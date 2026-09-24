# CPG Arena: 用遗传算法进化人造生物

一个轻量级的教学游戏。每个学生设计一只由方块和关节组成的 2D 生物，用 **CPG（中枢模式发生器）** 驱动关节，用 **遗传算法（GA）** 进化参数，再把生物文件放进同一个文件夹里，**赛跑**比谁跑得远，或者 **相扑**比谁把对手推下台。

| 赛跑（每只生物一条赛道） | 相扑（循环赛 + 决赛回放） |
|---|---|
| ![race](docs/race.png) | ![sumo](docs/sumo.png) |

源自本仓库的 MATLAB/Simulink 项目（Snake5 / chain_CPG），CPG 模型相同（Sproewitz et al. 2008），但修正了原 MATLAB 代码里的两个问题（见 [cpg.rs](src/cpg.rs) 顶部注释）。

---

## 快速开始

```bash
cd arena
cargo build --release              # 需要 Rust ≥ 1.92

# 打开界面，显示 creatures/ 文件夹里的所有生物
./target/release/arena-gui creatures
```

Linux 如果报 `libxkbcommon-x11.so could not be loaded`：`sudo apt install libxkbcommon-x11-0`。
在没有图形界面的服务器上，可以用 `cargo build --release --no-default-features` 只编译命令行工具。

---

## 游戏流程（学生）

```text
1. 设计身体           2. 进化                        3. 提交              4. 比赛
worm.toml  ──────▶  arena train ... --out me.toml ──▶ 拷进班级文件夹 ──▶ arena-gui 班级文件夹
（手写拓扑）        （GA 调参数，可选连身体一起进化）                    （自动刷新）
```

### 1. 设计身体：生物文件

一个 `.toml` 文件就是一只生物。`segment` 0 是躯干，其余每一节通过一个电机关节挂在前面的某一节上，每个关节由一个 CPG 振荡器驱动。

```toml
name = "Worm"
author = "张三"
color = [230, 120, 40]      # 可选

[brain]
frequency = 1.0             # 振荡频率 (Hz)，所有关节共用
coupling = 4.0              # 相邻振荡器同步的强度

[[segment]]                 # 0: 躯干，没有关节
length = 0.4                # 米
width = 0.12

[[segment]]
parent = 0                  # 挂在哪一节上（必须是前面的节）
attach = 1.0                # 挂在父节的哪里：-1 后端，0 中间，1 前端
angle = 0.0                 # 静止时相对父节的角度（度）
length = 0.4
width = 0.12
amplitude = 30.0            # 关节摆动幅度（度）
offset = 0.0                # 摆动中心的偏移（度）
phase = 90.0                # 这个关节的相位（度），关节之间的相位差决定步态
```

每个关节的角度是 `angle + offset + amplitude·cos(ψ)`，ψ 是 CPG 的相位。示例见 [`creatures/`](creatures)：`worm`（蠕虫）、`walker`（两条腿）、`tailfin`（尾巴），以及它们进化后的版本。

### 2. 进化

**方式 A：用内置 GA**

```bash
# 只进化 CPG 参数（频率、幅度、偏移、相位），身体不变
./target/release/arena train creatures/worm.toml --mode race --out creatures/my-worm.toml --name "我的虫"

# 连身体一起进化（各节长短粗细、挂接位置、角度），拓扑仍是你设计的
./target/release/arena train creatures/walker.toml --mode race --body --generations 40 --out ...

# 相扑：对着文件夹里的其他生物练（没有对手时对着一块石头练）
./target/release/arena train creatures/tailfin.toml --mode sumo --opponents creatures --out ...
```

可调参数：`--generations`、`--population`、`--sigma`（变异强度）、`--seed`。每一代的最好/平均适应度会写到 `<out>.history.csv`，可以拿来画收敛曲线。

**方式 B：自己写 GA（任何语言）**

模拟器是一个黑盒：输入 [0,1] 之间的一串基因，输出适应度。

```bash
./target/release/arena genes creatures/worm.toml          # 每个基因的含义和范围
./target/release/arena eval creatures/worm.toml --genes 0.3,0.5,... --mode race
# {"fitness": 12.3, "mode": "race", "name": "Worm"}
./target/release/arena eval ... --genes ... --out creatures/me.toml --name "我的冠军"
```

[`examples/my_ga.py`](examples/my_ga.py) 是一个只用 Python 标准库、约 60 行的最小 GA，可以从这里开始改选择、交叉和变异。

### 3. 比赛

```bash
./target/release/arena-gui 班级文件夹            # 界面：勾选生物 → Start race / Tournament
./target/release/arena-gui 班级文件夹 --race     # 打开就开始赛跑（投屏用）
./target/release/arena race 班级文件夹           # 命令行排行榜
./target/release/arena tournament 班级文件夹     # 命令行相扑循环赛
./target/release/arena check 班级文件夹          # 检查所有文件是否合规
```

界面每秒检查一次文件夹，有新文件或改动会自动重新加载。不合规的文件会在左侧用红字标出原因。

---

## 规则（教师）

所有生物遵守同一套规则，保证公平：**身体越大越重，但每个关节的电机都一样**。默认值在 [`creature.rs`](src/creature.rs) 的 `Rules::default()`，在班级文件夹里放一个 `arena.toml` 就能覆盖任意一项：

```toml
# arena.toml（只写要改的项）
max_segments = 8          # 最多几节
max_area = 0.6            # 身体总面积预算 (m²)
motor_torque = 5.0        # 每个关节的最大力矩 (N·m)
frequency = [0.2, 3.0]    # 频率范围 (Hz)
race_time = 15.0          # 赛跑时长 (s)
sumo_time = 20.0          # 相扑时长 (s)
ring_width = 8.0          # 相扑台宽度 (m)
friction = 0.9
```

| 模式 | 适应度 | 胜负 |
|---|---|---|
| 赛跑 | `race_time` 秒内质心向右移动的距离（米） | 距离排名 |
| 相扑 | 每个对手：胜 +1 / 平 0 / 负 −1，再加上"场地控制"差值 ∈ [−1, 1] | 掉下台即输；到时间则离中心更近者胜（差距太小算平）；循环赛每对打两场（左右各一次），胜 3 分、平 1 分 |

相扑适应度里的连续项是为了给 GA 一个平滑的梯度，否则大多数个体都是 0 分，进化很难起步。这本身就是一个值得在课上讨论的点：**适应度怎么设计，决定了进化出什么**。

---

## 代码结构

```text
arena/
├── src/
│   ├── cpg.rs        # CPG 振荡器网络（RK4），含单元测试
│   ├── creature.rs   # 生物文件格式、规则、合规检查、基因编解码
│   ├── sim.rs        # 2D 物理（rapier2d）：搭身体、驱动关节、赛道和相扑台
│   ├── ga.rs         # 实数编码 GA（锦标赛选择、BLX-α 交叉、高斯变异、精英保留），ask/tell 接口
│   ├── game.rs       # 适应度、训练循环、文件夹加载、循环赛
│   └── bin/
│       ├── arena.rs      # 命令行
│       └── arena-gui.rs  # 界面（eframe/egui）
├── creatures/        # 示例生物
└── examples/my_ga.py # 自己写 GA 的起点
```

`ga.rs` 用 ask/tell 接口，和生物无关：`ask()` 给出一批 [0,1]ⁿ 的基因，`tell(fitness)` 返回适应度。换成 CMA-ES、粒子群或者别的算法，只需要实现这两个方法。

模拟是确定性的：同一个文件、同一套规则，结果完全一样。在 4 核机器上，一次 15 秒赛跑约 10 ms，30 代 × 48 个体的训练约 4 秒。

## 课堂可以讨论的问题

- 只调 CPG 参数 vs 连身体一起进化，结果差多少？搜索空间变大了多少？
- 为赛跑进化的生物，相扑为什么常常打不过（反之亦然）？专才和通才。
- 相扑里经常出现"石头剪刀布"式的循环克制，没有绝对最强：这和协同进化有什么关系？
- GA 找到的"作弊"步态（比如翻个身滑行）：是 bug 还是创新？规则应该怎么改？
- 固定随机种子 vs 换种子，结果稳定吗？
