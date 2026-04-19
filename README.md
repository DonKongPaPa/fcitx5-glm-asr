# fcitx5-glm-asr

基于智谱 GLM ASR 的 Linux 语音输入 fcitx5 插件。

长按右 Ctrl 录音，松开后自动识别并输入文字。

## 功能

- 长按右 Ctrl 键录音，松开识别
- 候选框状态反馈（🎙录音中 / ⏳识别中 / ✅准备输入）
- fcitx5-configtool 图形化配置（API Key、模型、采样率）

## 安装

详见 [INSTALL.md](INSTALL.md)

## 配置

使用 `fcitx5-configtool` 打开插件管理，找到 **GLM ASR**，配置以下参数：

- **API Key** — 智谱开放平台 API Key
- **Model** — ASR 模型名称（默认 `glm-asr-2512`）
- **API URL** — API 端点地址（默认 `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`，一般无需修改）
- **Sample Rate** — 录音采样率（默认 16000 Hz）

## 许可证

[MIT](LICENSE)
