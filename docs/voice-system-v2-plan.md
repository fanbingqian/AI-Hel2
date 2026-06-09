# AI-Hel2 语音系统 V2 改造方案

## 一、方案概述

将当前"文件级半双工级联"语音系统升级为"流式全双工级联"架构，核心思路参考**豆包 + 百聆**的工程设计，全部使用 **Apache 2.0 / MIT 协议的开源组件**，可商用。

### 1.1 当前架构 vs 目标架构

```
【当前 V1 — 文件级半双工】
录音(300ms帧,WAV文件) → faster-whisper(批处理) → Hermes Agent(SSE) → edge-tts(MP3文件)
延迟: 5-12秒 | 判停: 音量阈值 | 打断: 不支持 | TTS自然度: 平淡

【目标 V2 — 流式全双工】
录音(30ms帧,音频流) → silero-vad → sherpa-onnx ASR(流式) → Hermes Agent(SSE) → CosyVoice 2.0(流式)
延迟: 1.5-3秒 | 判停: VAD+语义 | 打断: 支持(<300ms) | TTS自然度: 情感丰富
```

### 1.2 核心指标对比

| 指标 | V1 当前 | V2 目标 | 豆包(参考) |
|------|---------|---------|------------|
| 端到端延迟 | 5-12s | 1.5-3s | <200ms |
| 打断支持 | 不支持 | 支持(<300ms响应) | 支持(<170ms) |
| 中文ASR准确率 | ~85% (whisper small) | ~95% (sherpa-onnx zipformer) | ~97% |
| VAD判停方式 | 音量阈值(0.005) | silero-vad神经网络 | 语义级判停 |
| TTS自然度 | 平淡朗读 | 情感可控 | 情感丰富 |
| 全双工 | 半双工 | 软全双工(可打断) | 硬全双工(边听边说) |
| 许可证 | 混合(MIT+灰色edge-tts) | 全部Apache 2.0/MIT | 闭源 |

---

## 二、解决的问题

### 问题 1：ASR 识别不准、乱码

**现象**：转录文字出现 `���` 乱码，中文识别准确率不够，特别是口语化表达和噪音环境下。

**根因**：
1. Windows 上 Python stdout 默认 GBK 编码，中文文本传到 Rust 侧用 UTF-8 解析产生乱码
2. `faster-whisper small` (461MB) 对中文识别率有限，且没有语言模型做语义纠偏

**解决方案**：
- 已修复编码：`sys.stdout.reconfigure(encoding='utf-8')` + `PYTHONIOENCODING=utf-8`
- V2 换 **sherpa-onnx** 的 zipformer-ctc-zh 模型：
  - 111M 参数专为中文优化，识别率显著高于 whisper small
  - 支持热词增强（boosting table），可注入领域词汇提升准确率
  - 支持标点恢复，输出带标点的完整句子
  - Apache 2.0 协议，Rust 原生 API 不经过 Python，彻底消除编码问题

### 问题 2：TTS 只回复前几个字就断掉

**现象**：AI 回复一段话，TTS 只说了开头几个字就停止。

**根因**：
1. AI 回复包含 Markdown（`**加粗**`、代码块、链接），edge-tts 解析特殊字符时异常截断
2. edge-tts 对单次请求有隐式字符限制（约 300-500 字），长文被截
3. 没有按句子拆分，一次性发送整段文本

**解决方案**：
- 已修复 Markdown 剥离：`strip_markdown()` 在发送前清洗文本
- 已加 2000 字截断保护
- V2 换 **CosyVoice 2.0**：
  - 按标点自动分句，逐句流式合成，不受单次长度限制
  - 支持情感控制（开心/悲伤/严肃等），语音不再"平淡"
  - 首包延迟 150ms，支持流式播放
  - Apache 2.0 协议，可商用

### 问题 3：语音交互需要点两次（开始/停止）

**现象**：点击电话按钮→说话→再点击停止，需要手动控制开始和结束。

**根因**：录音没有自动判停机制（或判停太粗糙），需要人工判断说完。

**解决方案**：
- 已修复：录音脚本加入静音检测，连续 1.5s 无声自动停止
- V2 进一步优化：
  - **silero-vad** 替换音量阈值：基于神经网络的语音检测，准确区分人声/噪音
  - 帧大小从 300ms 降到 **30ms**，VAD 响应延迟从 300ms 降到 30ms
  - 支持配置判停策略：静音时长、最小语音时长、噪声阈值

### 问题 4：无法打断 AI 回复

**现象**：AI 正在朗读回复时，用户说话无法被打断和响应，必须等 AI 说完。

**根因**：当前架构是串行的——TTS 播放期间无法同时监听麦克风，Audio 元素播放是前端行为，后端不知情。

**解决方案（V2 全双工架构）**：
- TTS 流式播放期间，**麦克风持续监听**（并行音频管道）
- silero-vad 检测到用户语音 → 触发打断流程：
  1. 停止当前 TTS 播放（前端 stop + 后端清理）
  2. 调用 `abort_chat()` 断开当前 SSE 流（已有机制）
  3. 开启新录音→识别→LLM→TTS 管道
- 打断响应时间目标：< 300ms（百聆实测 800ms 端到端）

---

## 三、技术方案

### 3.1 组件选型

```
┌──────────────────────────────────────────────────────────┐
│                    音频输入 (麦克风)                       │
│                  30ms 帧, 16kHz mono                      │
└──────────────────────┬───────────────────────────────────┘
                       │
              ┌────────┴────────┐
              │   silero-vad    │  ← MIT 协议, ONNX 推理
              │   语音检测/判停  │    30ms 帧级响应
              └────────┬────────┘
                       │ 有效语音段
              ┌────────┴────────┐
              │  sherpa-onnx    │  ← Apache 2.0, Rust API
              │  zipformer-zh   │    zipformer CTC 中文模型
              │  流式 ASR        │    <500ms 识别延迟
              └────────┬────────┘
                       │ 识别文本
              ┌────────┴────────┐
              │  Hermes Agent   │  ← 已有, SSE 流式返回
              │  HTTP SSE API   │    cancel_flag 支持中止
              │  (模型层)       │
              └────────┬────────┘
                       │ 回复文本
              ┌────────┴────────┐
              │  CosyVoice 2.0  │  ← Apache 2.0, Python 子进程
              │  情感流式 TTS    │    150ms 首包, 逐句合成
              └────────┬────────┘
                       │
              ┌────────┴────────┐
              │  音频输出(扬声器) │
              │  PCM 流式播放    │
              └─────────────────┘
```

### 3.2 sherpa-onnx 集成（替代 faster-whisper）

**为什么选 sherpa-onnx：**
- 有官方 Rust API（`sherpa-rs`），不需要 Python 子进程，彻底消除编码问题
- zipformer-ctc-zh 模型专为中文优化，111M 参数，CPU 实时
- 自带 VAD 能力（也可配合独立 silero-vad）
- 流式识别 API，边录边识别，不需要等录音结束
- Apache 2.0 商用友好

**集成方式：**
```toml
# Cargo.toml
sherpa-rs = "0.3"   # sherpa-onnx Rust binding
```

```rust
// 流式 ASR 示例
use sherpa_rs::recognizer::Recognizer;

let mut recognizer = Recognizer::new(recognizer_config)?;
let stream = recognizer.create_stream()?;

// 每收到一个音频帧（30ms）
for audio_chunk in audio_rx.iter() {
    stream.accept_waveform(16000, &audio_chunk);
    while let Some(text) = stream.get_result() {
        // 流式返回识别文本
        emit_partial_result(&text);
    }
}
```

### 3.3 CosyVoice 2.0 集成（替代 edge-tts）

**为什么选 CosyVoice：**
- 中文 TTS 开源最佳，阿里达摩院维护
- Apache 2.0 协议，代码和模型均可商用
- 支持流式合成（首包 150ms），支持情感控制
- 3 秒音频即可克隆音色

**集成方式：**
```python
# cosyvoice_server.py — 独立的 TTS 服务进程
from cosyvoice.cli.cosyvoice import CosyVoice2

model = CosyVoice2('pretrained_models/CosyVoice2-0.5B', load_jit=False)

# 流式合成
for segment in model.inference_zero_shot(text, stream=True):
    # segment 是 PCM 音频数据
    sys.stdout.buffer.write(segment.tobytes())
```

Rust 侧通过子进程 stdin/stdout 管道通信，避免文件 I/O：
```rust
// Rust 侧 — 管道通信
let mut tts_process = Command::new("python")
    .args(["-m", "cosyvoice_server"])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()?;

// 写入文本
tts_process.stdin.write_all(text.as_bytes())?;

// 读取 PCM 音频数据，流式发送给前端
let mut pcm_buf = vec![0u8; 4096];
while let Ok(n) = tts_process.stdout.read(&mut pcm_buf) {
    if n == 0 { break; }
    emit_audio_chunk(&pcm_buf[..n]);
}
```

### 3.4 打断机制设计

```
                          ┌─────────────┐
     麦克风输入 ─────────→│  silero-vad  │(持续运行)
                          └──────┬──────┘
                                 │
                    ┌────────────┼────────────┐
                    │ 检测到语音  │ 检测到噪音  │ 检测到说话结束
                    └──────┬─────┴──────┬─────┴──────┬─────┘
                           │            │            │
                    ┌──────┴──────┐     │     ┌──────┴──────┐
                    │ 是否正在TTS?│     │     │ 开始/继续   │
                    └──┬──────┬──┘     │     │ ASR识别     │
                       │Yes   │No       │     └─────────────┘
                  ┌────┴──┐ ┌┴────────┐│
                  │打断!  │ │正常录音  ││
                  │1.stop │ │          ││
                  │ TTS   │ │          ││
                  │2.abort│ │          ││
                  │ SSE   │ │          ││
                  │3.新录音│ │          ││
                  │ 开始  │ │          ││
                  └───────┘ └──────────┘│
```

**打断核心流程（Rust 侧）：**
1. VAD 检测到语音开始 → 检查 `tts_playing` 状态标志
2. 如果 TTS 正在播放：
   - 设置 `cancel_flag = true` → 断开 SSE 流
   - 发送 `chat:interrupted` 事件给前端 → 停止 Audio 播放
   - 清空音频输出缓冲区
3. 开始新轮次录音
4. 判停后 → sherpa-onnx 识别 → 发送给 Hermes Agent
5. Agent 返回新回复 → CosyVoice 合成 → 播放

### 3.5 Rust 音频管道架构

```rust
pub struct VoicePipeline {
    vad: silero_vad::VadSession,
    asr: sherpa_rs::Recognizer,
    tts_process: Option<Child>,
    audio_output: Arc<Mutex<VecDeque<Vec<u8>>>>,
    state: Arc<AtomicU8>, // 0=idle, 1=listening, 2=speaking, 3=interrupted
}

impl VoicePipeline {
    /// 主循环：并行处理音频输入和输出
    pub async fn run(&mut self) {
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<i16>>(64);

        // 线程1: 音频采集 + VAD
        let vad_state = self.state.clone();
        tokio::spawn(async move {
            let stream = audio_input_stream();
            loop {
                let chunk = stream.read(30ms);
                let is_speech = vad.detect(&chunk);
                audio_tx.send((chunk, is_speech)).await;
            }
        });

        // 线程2: 主状态机
        loop {
            match self.state.load(Ordering::Relaxed) {
                0 => { /* idle: 等待VAD检测到语音 */ }
                1 => { /* listening: 积累音频 → ASR → 判停 */ }
                2 => { /* speaking: TTS播放中，监控VAD打断 */ }
                3 => { /* interrupted: 清理 → 回到listening */ }
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
```

---

## 四、实施计划

### Phase 1：快速修复（已完成）

- [x] 编码修复：`sys.stdout.reconfigure(encoding='utf-8')` + `PYTHONIOENCODING`
- [x] Markdown 剥离：`strip_markdown()` 函数
- [x] 录音帧优化 + 自动判停：静音 1.5s 自动停止
- [x] TTS 文本截断保护：2000 字 + 句尾截断
- [x] base64 传输替代 asset protocol

### Phase 2：sherpa-onnx 集成（1 周）

- [ ] 添加 `sherpa-rs` 依赖到 Cargo.toml
- [ ] 实现流式 ASR 模块：`src/services/asr_service.rs`
- [ ] 集成 silero-vad：ONNX 推理或独立 Python 微服务
- [ ] 替换 `stt_service.rs` 中的 faster-whisper 调用
- [ ] 测试中文识别准确率和延迟

### Phase 3：CosyVoice 2.0 集成（1 周）

- [ ] 部署 CosyVoice 2.0 模型（0.5B，约 3GB）
- [ ] 实现流式 TTS 服务：Python 子进程 + stdin/stdout 管道
- [ ] Rust 侧 PCM 流式读取和前端播放
- [ ] 替换 edge-tts 调用
- [ ] 配置中文音色、情感参数

### Phase 4：打断支持（1-2 周）

- [ ] 实现音频输入/输出并行管道
- [ ] VAD 持续监听 + 打断状态机
- [ ] 集成 `abort_chat()` 到打断流程
- [ ] 前端打断事件处理（停止 Audio、UI 状态切换）
- [ ] 端到端打断测试

### Phase 5：打磨与优化（持续）

- [ ] sherpa-onnx 热词配置（领域术语提升识别率）
- [ ] CosyVoice 音色选择和情感映射规则
- [ ] 不同环境下的 VAD 参数自适应
- [ ] 性能压测和内存优化

---

## 五、预期效果

### 5.1 性能指标

| 指标 | V1 当前 | V2 Phase 4 完成后 |
|------|---------|-------------------|
| 端到端延迟（说完→听到回复首音） | 5-12s | **1.5-3s** |
| ASR 中文准确率 | ~85% | **~95%** |
| 打断响应时间 | 不支持 | **<300ms** |
| TTS 首包延迟 | 1-2s(文件级) | **150ms**(流式) |
| TTS 自然度评分 | 3/5 | **4.5/5** |
| VAD 误判率 | ~30% | **<10%** |
| 编码问题 | 偶发乱码 | **完全消除** |

### 5.2 用户体验变化

| 场景 | V1 | V2 |
|------|-----|-----|
| 语音对话 | 点开始→说话→点停止→等待5-10s | **点一下→说话→自动识别→1.5-3s回复** |
| 打断AI | 必须等AI说完 | **直接说话打断，<300ms响应** |
| 长回复 | 被截断只说开头 | **逐句合成播放，完整回复** |
| AI声音 | 平淡朗读，像机器人 | **情感自然，接近真人** |
| 嘈杂环境 | 容易误触发 | **VAD 抗噪，准确判停** |

### 5.3 与豆包的差距

| 豆包特有 | 我们的取舍 |
|----------|-----------|
| 端到端 S2S 模型(<200ms) | 级联方案 1.5-3s（可接受） |
| 语义级判停 | VAD + 静音阈值（基本可用） |
| 硬全双工(边听边说) | 软全双工(可打断)（体验接近） |
| 情绪感知 TTS | CosyVoice 情感指令（近似效果） |
| 云端算力，毫秒级 | 本地 CPU，秒级（隐私优先） |

---

## 六、风险与应对

| 风险 | 概率 | 影响 | 应对 |
|------|------|------|------|
| CosyVoice 显存不足 | 中 | 延迟增加 | 0.5B 模型 CPU 可跑，或用 sherpa-onnx TTS 降级 |
| sherpa-onnx Rust API 不完善 | 低 | 集成困难 | 可回退 Python 子进程调用 |
| 打断导致 Hermes Agent 状态错乱 | 中 | 对话混乱 | 每次打断新建 session 或依赖已有 session 管理 |
| 音频管道并行导致竞态 | 中 | 崩溃/杂音 | 使用 Mutex + 状态机严格控制 |
