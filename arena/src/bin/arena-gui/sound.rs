//! Sound effects for the Cup tab: countdown beeps, the crowd, a drum roll, a
//! fanfare and applause. Everything is synthesised when the sound is first
//! needed, so there are no sound files to ship. Without an audio device (or
//! when built without the `sound` feature) every call does nothing.

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

    /// A cheering crowd: a roar of voices and dense clapping, swelling and fading.
    fn cheer(seconds: f32, rate: f32, seed: u32) -> Vec<f32> {
        let n = (seconds * rate) as usize;
        let mut noise = Noise::new(seed);
        let mut roar = Band::new(250.0, 1400.0, rate);
        let mut wobble = LowPass::new(3.0, rate);
        let mut out: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                let env = (t / 0.25).min(1.0) * ((seconds - t) / (seconds * 0.6)).clamp(0.0, 1.0);
                roar.run(noise.next()) * env * (0.7 + 3.0 * wobble.run(noise.next()).abs())
            })
            .collect();
        // "Wooo!": a few voices gliding up.
        for _ in 0..7 {
            let start = noise.unit() * seconds * 0.4;
            let len = 0.5 + noise.unit() * 0.8;
            let (f0, f1) = (350.0 + noise.unit() * 250.0, 700.0 + noise.unit() * 400.0);
            let mut phase = 0.0;
            let voice: Vec<f32> = (0..(len * rate) as usize)
                .map(|i| {
                    let u = i as f32 / (len * rate);
                    phase += (f0 + (f1 - f0) * u.sqrt()) / rate;
                    let env = (u / 0.15).min(1.0) * (1.0 - u).powf(1.5);
                    ((TAU * phase).sin() + 0.3 * (2.0 * TAU * phase).sin()) * env
                })
                .collect();
            mix(&mut out, &voice, (start * rate) as usize, 0.05);
        }
        // Clapping: many short, bright bursts.
        let mut band = Band::new(800.0, 3500.0, rate);
        let clap: Vec<f32> = (0..(0.03 * rate) as usize).map(|i| band.run(noise.next()) * (-(i as f32 / rate) / 0.008).exp()).collect();
        let claps = (seconds * 45.0) as usize;
        for _ in 0..claps {
            let t = noise.unit().powf(0.8) * seconds;
            let env = (t / 0.3).min(1.0) * ((seconds - t) / (seconds * 0.7)).clamp(0.0, 1.0);
            mix(&mut out, &clap, (t * rate) as usize, env * (0.3 + 0.7 * noise.unit()) * 2.2);
        }
        out
    }

    /// The murmur of a waiting crowd, seamless when looped.
    pub fn crowd(rate: f32) -> Vec<f32> {
        let (seconds, fade) = (6.0, 0.8);
        let n = (seconds * rate) as usize;
        let f = (fade * rate) as usize;
        let mut noise = Noise::new(99);
        let mut band = Band::new(200.0, 1100.0, rate);
        let mut wobble = LowPass::new(1.5, rate);
        let raw: Vec<f32> = (0..n + f).map(|_| band.run(noise.next()) * (0.6 + 4.0 * wobble.run(noise.next()).abs())).collect();
        // Cross-fade the extra tail into the start, so the loop has no seam.
        let mut out = raw[..n].to_vec();
        for i in 0..f {
            let u = i as f32 / f as f32;
            out[i] = raw[n + i] * (1.0 - u) + raw[i] * u;
        }
        normalise(out, 0.25)
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
    use std::sync::{Arc, Mutex};

    const CROWD: u32 = 1;

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
        bank: HashMap<Effect, Arc<Vec<f32>>>,
        crowd: Option<Arc<Vec<f32>>>,
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
            Some(Self { _stream: stream, voices, rate, bank: HashMap::new(), crowd: None })
        }

        fn add(&self, samples: Arc<Vec<f32>>, gain: f32, looping: bool, tag: u32) {
            if let Ok(mut v) = self.voices.lock() {
                let start = if looping { 0.0 } else { gain };
                v.push(Voice { samples, pos: 0, gain: start, target: gain, looping, tag });
            }
        }

        pub fn play(&mut self, e: Effect) {
            let rate = self.rate;
            let s = self.bank.entry(e).or_insert_with(|| Arc::new(synth::effect(e, rate))).clone();
            self.add(s, 1.0, false, 0);
        }

        pub fn crowd(&mut self, on: bool) {
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
