//! Sound effects for the Cup: countdown beeps, the crowd, a drum roll, a
//! fanfare and cheering. Everything is synthesised (in the background, when
//! the sound output opens), so there are no sound files to ship. Without an
//! audio device (or when built without the `sound` feature) every call does
//! nothing.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Effect {
    /// Countdown: 3, 2, 1 ...
    Beep,
    /// ... go!
    Go,
    /// Before the winner is announced.
    DrumRoll,
    /// The winner of a match.
    Fanfare,
    /// The winner of the cup.
    Champion,
    /// The crowd cheering (with the fanfares, and at the start).
    Cheer,
}

pub struct Sound {
    #[cfg(feature = "sound")]
    out: Option<imp::Output>,
    #[cfg(feature = "sound")]
    tried: bool,
    pub on: bool,
}

impl Sound {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "sound")]
            out: None,
            #[cfg(feature = "sound")]
            tried: false,
            on: true,
        }
    }

    #[cfg(feature = "sound")]
    fn output(&mut self) -> Option<&mut imp::Output> {
        if !self.on {
            return None;
        }
        if !self.tried {
            self.tried = true;
            self.out = imp::Output::open();
        }
        self.out.as_mut()
    }

    /// Open the sound output now, so that the sounds are made by the time the
    /// first match starts.
    pub fn prepare(&mut self) {
        #[cfg(feature = "sound")]
        let _ = self.output();
    }

    /// Play an effect once, over whatever is playing.
    pub fn play(&mut self, effect: Effect) {
        #[cfg(feature = "sound")]
        if let Some(out) = self.output() {
            out.play(effect);
        }
        #[cfg(not(feature = "sound"))]
        let _ = effect;
    }

    /// The murmur of the crowd during a match (fades in and out).
    pub fn crowd(&mut self, on: bool) {
        #[cfg(feature = "sound")]
        {
            if on {
                if let Some(out) = self.output() {
                    out.crowd(true);
                }
            } else if let Some(out) = self.out.as_mut() {
                out.crowd(false);
            }
        }
        #[cfg(not(feature = "sound"))]
        let _ = on;
    }

    /// Stop everything (fading out), e.g. when leaving a match.
    pub fn stop(&mut self) {
        #[cfg(feature = "sound")]
        if let Some(out) = self.out.as_mut() {
            out.stop();
        }
    }
}

/// The synthesised sounds, as mono samples at `rate` Hz. Kept apart from the
/// audio output so they can be tested without a sound card.
#[cfg_attr(not(feature = "sound"), allow(dead_code))]
pub mod synth {
    use super::Effect;
    use std::f32::consts::TAU;

    /// A small, fast noise generator (xorshift).
    pub struct Noise(u32);

    impl Noise {
        pub fn new(seed: u32) -> Self {
            Self(seed.max(1))
        }
        /// White noise in [-1, 1].
        pub fn next(&mut self) -> f32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x as f32 / u32::MAX as f32 * 2.0 - 1.0
        }
        /// Uniform in [0, 1).
        pub fn unit(&mut self) -> f32 {
            (self.next() + 1.0) / 2.0
        }
    }

    /// One-pole low-pass filter.
    struct LowPass {
        a: f32,
        y: f32,
    }

    impl LowPass {
        fn new(cutoff: f32, rate: f32) -> Self {
            Self { a: 1.0 - (-TAU * cutoff / rate).exp(), y: 0.0 }
        }
        fn run(&mut self, x: f32) -> f32 {
            self.y += self.a * (x - self.y);
            self.y
        }
    }

    /// Band of noise between two cut-offs (low-pass minus a lower low-pass).
    struct Band {
        hi: LowPass,
        lo: LowPass,
    }

    impl Band {
        fn new(low: f32, high: f32, rate: f32) -> Self {
            Self { hi: LowPass::new(high, rate), lo: LowPass::new(low, rate) }
        }
        fn run(&mut self, x: f32) -> f32 {
            let h = self.hi.run(x);
            h - self.lo.run(h)
        }
    }

    fn silence(seconds: f32, rate: f32) -> Vec<f32> {
        vec![0.0; (seconds * rate) as usize]
    }

    /// Scale to a peak of `peak`.
    fn normalise(mut s: Vec<f32>, peak: f32) -> Vec<f32> {
        let max = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if max > 0.0 {
            for v in &mut s {
                *v *= peak / max;
            }
        }
        s
    }

    /// Add `src` into `dst` from sample `at` on.
    fn mix(dst: &mut [f32], src: &[f32], at: usize, gain: f32) {
        for (d, s) in dst.iter_mut().skip(at).zip(src) {
            *d += s * gain;
        }
    }

    /// A clean tone with a soft attack and an exponential tail.
    fn tone(freqs: &[f32], seconds: f32, decay: f32, rate: f32) -> Vec<f32> {
        let n = (seconds * rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                let env = (t / 0.004).min(1.0) * (-t / decay).exp() * ((seconds - t) / 0.01).clamp(0.0, 1.0);
                env * freqs.iter().map(|f| (TAU * f * t).sin() + 0.25 * (2.0 * TAU * f * t).sin()).sum::<f32>()
            })
            .collect()
    }

    /// A brass-like note: a sawtooth-ish sum of harmonics with vibrato.
    fn brass(freq: f32, seconds: f32, rate: f32) -> Vec<f32> {
        let n = (seconds * rate) as usize;
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                let vibrato = 1.0 + 0.006 * (TAU * 5.5 * t).sin() * (t / 0.25).min(1.0);
                phase += freq * vibrato / rate;
                // Brighter as the note swells, like a real trumpet.
                let bright = 0.5 + 0.5 * (t / 0.06).min(1.0);
                let wave: f32 = (1..=8).map(|k| (TAU * phase * k as f32).sin() * bright.powi(k as i32 - 1) / k as f32).sum();
                let env = (t / 0.03).min(1.0) * (0.85 + 0.15 * (-t / 0.2).exp()) * ((seconds - t) / 0.06).clamp(0.0, 1.0);
                wave * env
            })
            .collect()
    }

    /// A tune of (semitones above middle C, beats); two voices a third apart.
    fn fanfare(notes: &[(i32, f32)], beat: f32, rate: f32) -> Vec<f32> {
        let total: f32 = notes.iter().map(|n| n.1).sum::<f32>() * beat + 0.4;
        let mut out = silence(total, rate);
        let mut at = 0.0;
        for &(semi, beats) in notes {
            let f = 261.63 * 2f32.powf(semi as f32 / 12.0);
            let len = beats * beat;
            let start = (at * rate) as usize;
            mix(&mut out, &brass(f, len * 0.92 + 0.08, rate), start, 1.0);
            mix(&mut out, &brass(f * 2f32.powf(-4.0 / 12.0), len * 0.92 + 0.08, rate), start, 0.45);
            at += len;
        }
        out
    }

    /// One snare hit: a noise burst with a little drum body.
    fn snare(rate: f32, noise: &mut Noise) -> Vec<f32> {
        let n = (0.09 * rate) as usize;
        let mut band = Band::new(900.0, 7000.0, rate);
        (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                band.run(noise.next()) * (-t / 0.03).exp() * 1.6 + (TAU * 190.0 * t).sin() * (-t / 0.02).exp() * 0.5
            })
            .collect()
    }

    /// Accelerating, swelling snare roll that ends in a cymbal crash.
    fn drum_roll(rate: f32) -> Vec<f32> {
        let length = 2.1;
        let mut out = silence(length + 1.6, rate);
        let mut noise = Noise::new(7);
        let mut t = 0.0;
        while t < length {
            let u = t / length;
            let hit = snare(rate, &mut noise);
            mix(&mut out, &hit, (t * rate) as usize, 0.25 + 0.75 * u * u);
            t += 0.075 - 0.04 * u + 0.006 * noise.next();
        }
        // The crash.
        let mut band = Band::new(3000.0, 12000.0, rate);
        let crash: Vec<f32> = (0..(1.6 * rate) as usize).map(|i| band.run(noise.next()) * (-(i as f32 / rate) / 0.45).exp() * 2.5).collect();
        mix(&mut out, &crash, (length * rate) as usize, 1.0);
        out
    }

    // ------------------------------------------------------------ the crowd
    //
    // A crowd is many people: each voice is a buzz (the vocal folds) shaped
    // by the resonances of the mouth (the formants of a vowel), and each pair
    // of hands claps at its own pace. Many of them together, with a little
    // echo of the stadium, sound like a crowd and not like noise.

    /// Two-pole resonator: lets through a band of `bandwidth` Hz around `freq`,
    /// with a gain of about 1 there.
    struct Resonator {
        b0: f32,
        a1: f32,
        a2: f32,
        y1: f32,
        y2: f32,
    }

    impl Resonator {
        fn new(freq: f32, bandwidth: f32, rate: f32) -> Self {
            let mut r = Self { b0: 0.0, a1: 0.0, a2: 0.0, y1: 0.0, y2: 0.0 };
            r.tune(freq, bandwidth, rate);
            r
        }
        /// Move the band (the filter keeps ringing smoothly).
        fn tune(&mut self, freq: f32, bandwidth: f32, rate: f32) {
            let r = (-std::f32::consts::PI * bandwidth / rate).exp();
            let theta = TAU * freq.min(rate * 0.45) / rate;
            (self.b0, self.a1, self.a2) = ((1.0 - r) * 2.0 * theta.sin(), 2.0 * r * theta.cos(), -r * r);
        }
        fn run(&mut self, x: f32) -> f32 {
            let y = self.b0 * x + self.a1 * self.y1 + self.a2 * self.y2;
            (self.y2, self.y1) = (self.y1, y);
            y
        }
    }

    /// A sawtooth (`phase` in [0, 1), `dt` its step), smoothed at the jump so it
    /// does not alias: the buzz of a voice.
    fn saw(phase: f32, dt: f32) -> f32 {
        let blep = if phase < dt {
            let t = phase / dt;
            t + t - t * t - 1.0
        } else if phase > 1.0 - dt {
            let t = (phase - 1.0) / dt;
            t * t + t + t + 1.0
        } else {
            0.0
        };
        2.0 * phase - 1.0 - blep
    }

    /// A vowel by its first three formants (Hz), as shouted.
    #[derive(Clone, Copy)]
    struct Vowel([f32; 3]);
    const I: Vowel = Vowel([320.0, 2200.0, 2950.0]);
    const E: Vowel = Vowel([560.0, 1850.0, 2600.0]);
    const AE: Vowel = Vowel([760.0, 1700.0, 2550.0]);
    const A: Vowel = Vowel([820.0, 1250.0, 2600.0]);
    const O: Vowel = Vowel([560.0, 920.0, 2450.0]);
    const U: Vowel = Vowel([370.0, 820.0, 2300.0]);
    const VOWELS: [Vowel; 6] = [I, E, AE, A, O, U];

    /// One person saying or shouting one syllable.
    struct Utterance {
        seconds: f32,
        /// Pitch (Hz) at the start, at the top (`top` of the way through) and at the end.
        pitch: [f32; 3],
        top: f32,
        /// The vowel glides from the first to the second.
        vowels: [Vowel; 2],
        /// Seconds to full voice.
        attack: f32,
        /// How breathy (0: a pure buzz).
        breath: f32,
    }

    fn utter(u: &Utterance, rate: f32, noise: &mut Noise) -> Vec<f32> {
        let n = (u.seconds * rate) as usize;
        let bandwidths = [110.0, 140.0, 190.0];
        let gains = [1.0, 0.75, 0.45];
        // A raised voice is bright: more of the upper formants (pre-emphasis).
        let mut last = 0.0f32;
        let mut formants = u.vowels[0].0.map(|_| Resonator::new(500.0, 100.0, rate));
        let mut wander = LowPass::new(3.0, rate);
        let (vibrato, vibrato_phase) = (4.5 + 2.0 * noise.unit(), TAU * noise.unit());
        let mut phase = noise.unit();
        (0..n)
            .map(|i| {
                let (t, x) = (i as f32 / rate, i as f32 / n as f32);
                if i % 32 == 0 {
                    let g = (x / 0.6).min(1.0);
                    let g = g * g * (3.0 - 2.0 * g);
                    for (k, f) in formants.iter_mut().enumerate() {
                        f.tune(u.vowels[0].0[k] + (u.vowels[1].0[k] - u.vowels[0].0[k]) * g, bandwidths[k], rate);
                    }
                }
                let p = if x < u.top { u.pitch[0] + (u.pitch[1] - u.pitch[0]) * x / u.top } else { u.pitch[1] + (u.pitch[2] - u.pitch[1]) * (x - u.top) / (1.0 - u.top) };
                let f0 = p * (1.0 + 0.012 * (TAU * vibrato * t + vibrato_phase).sin() + 3.0 * wander.run(noise.next()));
                let dt = f0 / rate;
                phase = (phase + dt).fract();
                let source = saw(phase, dt) + u.breath * noise.next();
                let voice: f32 = formants.iter_mut().zip(gains).map(|(f, g)| f.run(source) * g).sum();
                let bright = voice - 0.5 * last;
                last = voice;
                let fall = ((1.0 - x) / 0.35).min(1.0);
                bright * (t / u.attack).min(1.0) * fall * fall
            })
            .collect()
    }

    /// Someone cheering: "yeah!", "woo!", "aah!" or "hey!", in a high or a low voice.
    fn shout(noise: &mut Noise) -> Utterance {
        let base = if noise.unit() < 0.5 { 260.0 + 170.0 * noise.unit() } else { 140.0 + 110.0 * noise.unit() };
        let r = noise.unit();
        match (noise.unit() * 4.0) as u32 {
            0 => Utterance { seconds: 0.6 + 0.7 * r, pitch: [base, base * 1.25, base * 0.9], top: 0.3, vowels: [I, AE], attack: 0.04, breath: 0.25 },
            1 => Utterance { seconds: 0.8 + 1.0 * r, pitch: [base * 0.9, base * 1.6, base * 1.3], top: 0.55, vowels: [U, U], attack: 0.08, breath: 0.3 },
            2 => Utterance { seconds: 1.0 + 1.3 * r, pitch: [base, base * 1.1, base * 0.85], top: 0.3, vowels: [A, AE], attack: 0.06, breath: 0.3 },
            _ => Utterance { seconds: 0.25 + 0.2 * r, pitch: [base * 1.1, base * 1.2, base * 0.95], top: 0.3, vowels: [E, AE], attack: 0.02, breath: 0.2 },
        }
    }

    /// Someone talking to a neighbour: a short syllable in a speaking voice.
    fn chatter(noise: &mut Noise) -> Utterance {
        let base = if noise.unit() < 0.5 { 190.0 + 90.0 * noise.unit() } else { 105.0 + 60.0 * noise.unit() };
        let vowel = |noise: &mut Noise| VOWELS[(noise.unit() * VOWELS.len() as f32) as usize % VOWELS.len()];
        Utterance {
            seconds: 0.12 + 0.3 * noise.unit(),
            pitch: [base, base * (1.0 + 0.15 * noise.unit()), base * (0.85 + 0.1 * noise.unit())],
            top: 0.4,
            vowels: [vowel(noise), vowel(noise)],
            attack: 0.03,
            breath: 0.35,
        }
    }

    /// One clap: a crack of noise with the ring of cupped hands.
    fn clap(rate: f32, noise: &mut Noise) -> Vec<f32> {
        let mut ring = Resonator::new(900.0 + 1300.0 * noise.unit(), 350.0 + 400.0 * noise.unit(), rate);
        let decay = 0.004 + 0.006 * noise.unit();
        (0..(0.05 * rate) as usize)
            .map(|i| {
                let x = noise.next();
                (ring.run(x) + 0.25 * x) * (-(i as f32 / rate) / decay).exp()
            })
            .collect()
    }

    /// `people` clapping, each at their own pace, as loud as `loudness(t)`.
    fn applause(seconds: f32, people: usize, rate: f32, noise: &mut Noise, loudness: impl Fn(f32) -> f32) -> Vec<f32> {
        let mut out = silence(seconds, rate);
        let claps: Vec<Vec<f32>> = (0..12).map(|_| clap(rate, noise)).collect();
        for _ in 0..people {
            let period = 1.0 / (3.5 + 2.5 * noise.unit());
            let gain = 0.4 + 0.6 * noise.unit();
            let mut t = noise.unit() * period;
            while t < seconds {
                let g = loudness(t) * gain;
                if g > 0.01 {
                    mix(&mut out, &claps[(noise.unit() * 12.0) as usize % 12], (t * rate) as usize, g);
                }
                t += period * (1.0 + 0.08 * noise.next());
            }
        }
        out
    }

    /// A finger whistle: up, held, then sliding down.
    fn whistle(seconds: f32, rate: f32, noise: &mut Noise) -> Vec<f32> {
        let n = (seconds * rate) as usize;
        let f = 2300.0 + 900.0 * noise.unit();
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let (t, x) = (i as f32 / rate, i as f32 / n as f32);
                let glide = if x < 0.15 { 0.8 + 0.2 * x / 0.15 } else if x < 0.7 { 1.0 } else { 1.0 - 0.25 * (x - 0.7) / 0.3 };
                phase = (phase + f * glide * (1.0 + 0.004 * (TAU * 6.0 * t).sin()) / rate).fract();
                ((TAU * phase).sin() + 0.08 * noise.next()) * (t / 0.03).min(1.0) * ((1.0 - x) / 0.1).min(1.0)
            })
            .collect()
    }

    /// `count` utterances made by `make`, starting at times from `when`, mixed together.
    fn voices(seconds: f32, count: usize, rate: f32, noise: &mut Noise, make: fn(&mut Noise) -> Utterance, when: impl Fn(&mut Noise) -> f32) -> Vec<f32> {
        let mut out = silence(seconds, rate);
        for _ in 0..count {
            let start = when(noise);
            let u = make(noise);
            let gain = 0.5 + 0.5 * noise.unit();
            mix(&mut out, &utter(&u, rate, noise), (start * rate) as usize, gain);
        }
        out
    }

    /// Root mean square.
    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    /// Mix layers at the given loudness (RMS), whatever their own level.
    fn layers(parts: &[(&[f32], f32)]) -> Vec<f32> {
        let mut out = vec![0.0; parts.iter().map(|p| p.0.len()).max().unwrap_or(0)];
        for (s, level) in parts {
            mix(&mut out, s, 0, level / rms(s).max(1e-9));
        }
        out
    }

    /// The echo of the stadium (Schroeder: four combs, then two all-passes), mixed in.
    fn stadium(dry: &[f32], rate: f32, wet: f32) -> Vec<f32> {
        let mut echo = vec![0.0f32; dry.len()];
        for seconds in [0.0297, 0.0371, 0.0411, 0.0437] {
            let mut line = vec![0.0f32; (seconds * rate) as usize];
            for (k, (&x, e)) in dry.iter().zip(echo.iter_mut()).enumerate() {
                let slot = k % line.len();
                let y = line[slot];
                line[slot] = x + 0.78 * y;
                *e += 0.25 * y;
            }
        }
        for seconds in [0.005, 0.0017] {
            let mut line = vec![0.0f32; (seconds * rate) as usize];
            for (k, e) in echo.iter_mut().enumerate() {
                let slot = k % line.len();
                let y = line[slot] - 0.7 * *e;
                line[slot] = *e + 0.7 * y;
                *e = y;
            }
        }
        dry.iter().zip(&echo).map(|(d, e)| d + wet * e).collect()
    }

    /// A cheering crowd: a roar of "yeah!" and "woo!", applause and a
    /// whistle or two, swelling at once and dying away.
    fn cheer(seconds: f32, rate: f32, seed: u32) -> Vec<f32> {
        let mut noise = Noise::new(seed);
        // Most people shout at once, a few a little later.
        let roar = voices(seconds, 60, rate, &mut noise, shout, |n| if n.unit() < 0.75 { 0.5 * n.unit() * n.unit() } else { 0.3 + 1.6 * n.unit() });
        let clapping = applause(seconds, 40, rate, &mut noise, |t| (t / 0.4).min(1.0) * ((seconds - t) / (seconds * 0.75)).clamp(0.0, 1.0));
        let mut whistles = silence(seconds, rate);
        for _ in 0..2 {
            let w = whistle(0.6 + 0.4 * noise.unit(), rate, &mut noise);
            mix(&mut whistles, &w, ((0.2 + 1.5 * noise.unit()) * rate) as usize, 1.0);
        }
        let mut out = stadium(&layers(&[(&roar, 1.0), (&clapping, 0.4), (&whistles, 0.12)]), rate, 0.35);
        // Die away at the end.
        let n = out.len() as f32;
        for (i, v) in out.iter_mut().enumerate() {
            *v *= ((n - i as f32) / (0.5 * rate)).min(1.0);
        }
        out
    }

    /// A crowd watching a match: people talking, someone shouting now and
    /// then, a few claps; seamless when looped.
    pub fn crowd(rate: f32) -> Vec<f32> {
        let (seconds, fade) = (6.0, 0.8);
        let total = seconds + fade;
        let mut noise = Noise::new(99);
        let talk = voices(total, 600, rate, &mut noise, chatter, |n| n.unit() * total);
        let shouts = voices(total, 14, rate, &mut noise, shout, |n| n.unit() * total);
        let clapping = applause(total, 6, rate, &mut noise, |_| 1.0);
        let raw = stadium(&layers(&[(&talk, 1.0), (&shouts, 0.45), (&clapping, 0.1)]), rate, 0.35);
        // Cross-fade the extra tail into the start, so the loop has no seam.
        let (n, f) = ((seconds * rate) as usize, (fade * rate) as usize);
        let mut out = raw[..n].to_vec();
        for i in 0..f {
            let u = i as f32 / f as f32;
            out[i] = raw[n + i] * (1.0 - u) + raw[i] * u;
        }
        normalise(out, 0.3)
    }

    /// One effect.
    pub fn effect(e: Effect, rate: f32) -> Vec<f32> {
        match e {
            Effect::Beep => normalise(tone(&[880.0], 0.22, 0.09, rate), 0.5),
            Effect::Go => normalise(tone(&[880.0, 1318.5, 1760.0], 0.6, 0.25, rate), 0.6),
            Effect::DrumRoll => normalise(drum_roll(rate), 0.7),
            // G G G C, then the C chord up to a long high C.
            Effect::Fanfare => normalise(fanfare(&[(7, 0.5), (7, 0.5), (7, 0.5), (12, 1.5), (7, 0.5), (12, 0.5), (16, 0.5), (19, 2.5)], 0.16, rate), 0.6),
            Effect::Champion => normalise(
                fanfare(
                    &[(7, 0.5), (7, 0.5), (7, 0.5), (12, 1.5), (7, 0.5), (12, 0.5), (16, 1.0), (14, 0.5), (12, 0.5), (16, 0.5), (19, 1.0), (24, 4.0)],
                    0.17,
                    rate,
                ),
                0.65,
            ),
            Effect::Cheer => normalise(cheer(4.5, rate, 31), 0.45),
        }
    }
}

#[cfg(feature = "sound")]
mod imp {
    use super::{synth, Effect};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
    use std::collections::HashMap;
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex};

    const CROWD: u32 = 1;

    type Bank = HashMap<Effect, Arc<Vec<f32>>>;

    struct Voice {
        samples: Arc<Vec<f32>>,
        pos: usize,
        gain: f32,
        /// The gain glides here; a voice that fades to 0 ends.
        target: f32,
        looping: bool,
        tag: u32,
    }

    type Voices = Arc<Mutex<Vec<Voice>>>;

    pub struct Output {
        _stream: Stream,
        voices: Voices,
        rate: f32,
        bank: Bank,
        crowd: Option<Arc<Vec<f32>>>,
        /// Every sound, being made in the background (the crowd takes a moment).
        pending: Option<Receiver<(Bank, Arc<Vec<f32>>)>>,
    }

    impl Output {
        pub fn open() -> Option<Self> {
            let device = cpal::default_host().default_output_device()?;
            let config = device.default_output_config().ok()?;
            let voices: Voices = Arc::new(Mutex::new(Vec::new()));
            let format = config.sample_format();
            let config: StreamConfig = config.into();
            let rate = config.sample_rate as f32;
            let stream = match format {
                SampleFormat::F32 => build::<f32>(&device, config, voices.clone()),
                SampleFormat::I16 => build::<i16>(&device, config, voices.clone()),
                SampleFormat::U16 => build::<u16>(&device, config, voices.clone()),
                SampleFormat::I32 => build::<i32>(&device, config, voices.clone()),
                SampleFormat::F64 => build::<f64>(&device, config, voices.clone()),
                _ => None,
            }?;
            stream.play().ok()?;
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let effects = [Effect::Beep, Effect::Go, Effect::DrumRoll, Effect::Fanfare, Effect::Champion, Effect::Cheer];
                let bank = effects.into_iter().map(|e| (e, Arc::new(synth::effect(e, rate)))).collect();
                tx.send((bank, Arc::new(synth::crowd(rate)))).ok();
            });
            Some(Self { _stream: stream, voices, rate, bank: HashMap::new(), crowd: None, pending: Some(rx) })
        }

        /// Take the sounds made in the background, once they are ready.
        fn collect(&mut self) {
            let Some(rx) = &self.pending else { return };
            match rx.try_recv() {
                Ok((bank, crowd)) => {
                    self.bank.extend(bank);
                    self.crowd = Some(crowd);
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }

        fn add(&self, samples: Arc<Vec<f32>>, gain: f32, looping: bool, tag: u32) {
            if let Ok(mut v) = self.voices.lock() {
                let start = if looping { 0.0 } else { gain };
                v.push(Voice { samples, pos: 0, gain: start, target: gain, looping, tag });
            }
        }

        pub fn play(&mut self, e: Effect) {
            self.collect();
            let rate = self.rate;
            let s = self.bank.entry(e).or_insert_with(|| Arc::new(synth::effect(e, rate))).clone();
            self.add(s, 1.0, false, 0);
        }

        pub fn crowd(&mut self, on: bool) {
            self.collect();
            let Ok(mut v) = self.voices.lock() else { return };
            let mut playing = false;
            for voice in v.iter_mut().filter(|v| v.tag == CROWD && v.target > 0.0) {
                voice.target = if on { 1.0 } else { 0.0 };
                playing = true;
            }
            drop(v);
            if on && !playing {
                let rate = self.rate;
                let s = self.crowd.get_or_insert_with(|| Arc::new(synth::crowd(rate))).clone();
                self.add(s, 1.0, true, CROWD);
            }
        }

        pub fn stop(&mut self) {
            if let Ok(mut v) = self.voices.lock() {
                for voice in v.iter_mut() {
                    voice.target = 0.0;
                }
            }
        }
    }

    fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: StreamConfig, voices: Voices) -> Option<Stream> {
        let channels = config.channels as usize;
        // About 0.3 s to fade in or out.
        let glide = 1.0 / (0.3 * config.sample_rate as f32);
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    let mut voices = voices.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(channels) {
                        let mut sum = 0.0;
                        for v in voices.iter_mut() {
                            if v.pos >= v.samples.len() {
                                if !v.looping {
                                    continue;
                                }
                                v.pos = 0;
                            }
                            v.gain += (v.target - v.gain).clamp(-glide, glide);
                            sum += v.samples[v.pos] * v.gain;
                            v.pos += 1;
                        }
                        // Soft clipping keeps loud moments from crackling.
                        let value = T::from_sample(sum.tanh() * 0.9);
                        for s in frame.iter_mut() {
                            *s = value;
                        }
                    }
                    voices.retain(|v| (v.looping || v.pos < v.samples.len()) && (v.target > 0.0 || v.gain > 0.0));
                },
                |_| {},
                None,
            )
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::synth::{self, Noise};
    use super::Effect;

    #[test]
    fn every_effect_is_a_clean_sound() {
        let rate = 48_000.0;
        for e in [Effect::Beep, Effect::Go, Effect::DrumRoll, Effect::Fanfare, Effect::Champion, Effect::Cheer] {
            let s = synth::effect(e, rate);
            assert!(s.len() > (0.1 * rate) as usize, "{e:?} too short");
            assert!(s.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{e:?} clips");
            assert!(s.iter().any(|v| v.abs() > 0.1), "{e:?} is silent");
        }
        // The fanfare of the cup winner is the longer one.
        assert!(synth::effect(Effect::Champion, rate).len() > synth::effect(Effect::Fanfare, rate).len());
    }

    #[cfg(feature = "sound")]
    #[test]
    #[ignore = "needs an audio device"]
    fn the_audio_device_opens() {
        // Opens the output stream (silent: nothing is played).
        assert!(super::imp::Output::open().is_some());
    }

    /// Writes every sound as a .wav file to listen to:
    /// `cargo test --release --bin arena-cup -- --ignored write_the_sounds`
    #[test]
    #[ignore = "writes files to listen to"]
    fn write_the_sounds() {
        let rate = 48_000u32;
        let dir = std::env::temp_dir().join("arena-cup-sounds");
        std::fs::create_dir_all(&dir).unwrap();
        let mut all = vec![("crowd".to_string(), synth::crowd(rate as f32))];
        for e in [Effect::Beep, Effect::Go, Effect::DrumRoll, Effect::Fanfare, Effect::Champion, Effect::Cheer] {
            all.push((format!("{e:?}").to_lowercase(), synth::effect(e, rate as f32)));
        }
        for (name, s) in all {
            let data: Vec<u8> = s.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
            let mut wav = Vec::new();
            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            for field in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), rate.to_le_bytes().to_vec(), (rate * 2).to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
                wav.extend_from_slice(&field);
            }
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
            wav.extend_from_slice(&data);
            std::fs::write(dir.join(format!("{name}.wav")), wav).unwrap();
        }
        println!("written to {}", dir.display());
    }

    #[test]
    fn the_crowd_loops_without_a_click() {
        let s = synth::crowd(48_000.0);
        let jump = (s[s.len() - 1] - s[0]).abs();
        let typical = s.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / s.len() as f32;
        assert!(jump < typical * 20.0, "seam {jump} vs typical step {typical}");
        let mut n = Noise::new(1);
        assert!((0..1000).map(|_| n.next()).all(|v| (-1.0..=1.0).contains(&v)));
    }
}
