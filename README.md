# VoxInk

<p align="center">
  <strong>声落成墨，让 AI 提示词快人一步。</strong><br>
  <em>Speak your prompts, ink your thoughts.</em>
</p>

<p align="center">
  <a href="https://github.com/your-username/VoxInk/releases"><img src="https://img.shields.io/github/v/release/your-username/VoxInk?include_prereleases&style=flat-square" alt="Release"></a>
  <img src="https://img.shields.io/badge/Language-Rust-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/GUI-GPUI-blueviolet?style=flat-square" alt="GPUI">
  <img src="https://img.shields.io/badge/License-Apache_2.0-blue?style=flat-square" alt="License">
</p>

## 📖 项目简介

在日常使用大语言模型（LLM）时，复杂的提示词（Prompt）往往需要耗费大量的键盘输入与修改时间。**VoxInk** 是一款专为大模型高频使用者设计的开源、轻量级桌面语音提示词辅助工具。

得益于 **Rust** 语言与现代化 **GPUI** 框架，VoxInk 拥有极快的响应速度与极低的系统资源占用。它能够作为后台助手静默守护在您的系统托盘中，随时通过语音输入、实时/离线转录以及便捷的手动编辑，帮助您将脑海中的灵感快速“落笔成墨”，无缝输出至大模型对话框中。

## ✨ 核心特性

- 🎙️ **灵活的语音转录模式**：
  - **实时转录**：开启实时流式 ASR，边说边录，文本即时呈现在输入框中，适合发散性思维的即兴表达。
  - **离线转录**：仅在后台进行本地录音，待录音结束后一次性上传并完成整段高精度音频转录，适合更有条理的段落输入。
- 📝 **即时编辑与一键复制**：转录后的文本会被渲染在主界面的精简文本框中，支持任意的手动修改与校对，并提供一键复制功能，确保最终输入给大模型的提示词准确无误。
- ⚡ **轻量高效的运行体验**：采用 GPU 加速的 **GPUI** 框架构建，界面渲染顺滑，资源占用极小。
- 📥 **开机自启与托盘常驻**：支持开机自动启动，启动后默认最小化至系统右下角托盘，双击或单击托盘图标即可快速唤起/隐藏主界面，不打乱您原有的工作流。

## 🛠️ 技术栈

- **[Rust](https://rust-lang.org)**：A language empowering everyone to build reliable and efficient software.
- **[gpui](https://www.gpui.rs)**：A fast, productive UI framework for Rust from the creators of Zed.
- **[gpui-component](https://longbridge.github.io/gpui-component)**：Rust GUI components for building fantastic
  cross-platform desktop application by using GPUI.
- **语音识别 (ASR)**：本地离线ASR + 外部ASR服务多种模式可选

## 🚀 快速开始

### 1. 安装依赖

请确保系统已安装基本的音频输入驱动以及 Rust 编译环境。

### 2. 构建与运行

```bash
# 克隆仓库
git clone https://github.com/humbinal/VoxInk.git
cd VoxInk

# 编译并运行
cargo run --release
```

## 🤝 参与贡献

这是一个开源项目，非常欢迎感兴趣的开发者提交 Issue 或 Pull Request。如果您在使用过程中遇到任何问题，或者有关于功能设计的新想法，请随时与我联系。

## 📄 开源协议

本项目采用 [Apache License 2.0](LICENSE) 开源协议。
