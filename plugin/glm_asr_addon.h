#pragma once

#include <fcitx/addoninstance.h>
#include <fcitx-config/configuration.h>
#include <fcitx-config/enum.h>
#include <fcitx-config/option.h>
#include <fcitx-config/iniparser.h>
#include <fcitx-config/rawconfig.h>
#include <fcitx-utils/i18n.h>
#include <fcitx-utils/handlertable.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx-utils/key.h>
#include <fcitx/instance.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/event.h>
#include <fcitx/candidatelist.h>

#include <memory>
#include <string>
#include <vector>

enum class SampleRate { R8000, R16000, R44100, R48000 };
FCITX_CONFIG_ENUM_NAME_WITH_I18N(SampleRate, N_("8000"), N_("16000"), N_("44100"), N_("48000"))

enum class TriggerMode { Hold, Toggle };
FCITX_CONFIG_ENUM_NAME_WITH_I18N(TriggerMode, N_("Hold"), N_("Toggle"))

enum class OverlayRenderer { Software, Vello };
FCITX_CONFIG_ENUM_NAME_WITH_I18N(OverlayRenderer, N_("Software"), N_("Vello (Experimental)"))

FCITX_CONFIGURATION(
    GlmAsrConfig,
    fcitx::KeyListOption triggerKey{
        this,
        "TriggerKey",
        "Record Trigger Key",
        {fcitx::Key("Control_R")},
        fcitx::KeyListConstrain({fcitx::KeyConstrainFlag::AllowModifierOnly,
                                 fcitx::KeyConstrainFlag::AllowModifierLess})};
    fcitx::OptionWithAnnotation<TriggerMode, TriggerModeI18NAnnotation> triggerMode{
        this,
        "TriggerMode",
        "Trigger Mode",
        TriggerMode::Hold};
    fcitx::Option<std::string> apiKey{
        this,
        "ApiKey",
        "ASR API Key",
        std::string()};
    fcitx::Option<std::string> model{
        this,
        "Model",
        "ASR Model",
        std::string("glm-asr-2512")};
    fcitx::Option<std::string> apiUrl{
        this,
        "ApiUrl",
        "ASR API URL",
        std::string("https://open.bigmodel.cn/api/paas/v4/audio/transcriptions")};
    fcitx::OptionWithAnnotation<SampleRate, SampleRateI18NAnnotation> sampleRate{
        this,
        "SampleRate",
        "Sample Rate (Hz)",
        SampleRate::R16000};
    fcitx::Option<bool> useOverlay{
        this,
        "UseOverlay",
        "Use Overlay Window",
        true};
    fcitx::Option<bool> usePreedit{
        this,
        "UsePreedit",
        "Use Preedit (Inline Preview)",
        true};
    fcitx::OptionWithAnnotation<OverlayRenderer, OverlayRendererI18NAnnotation> overlayRenderer{
        this,
        "OverlayRenderer",
        "Overlay Renderer",
        OverlayRenderer::Software};

    fcitx::Option<bool> enableLlm{
        this,
        "EnableLlm",
        "Enable LLM Post-processing (model must support JSON structured output)",
        false};
    fcitx::Option<std::string> llmApiKey{
        this,
        "LlmApiKey",
        "LLM API Key (leave empty to use ASR API Key)",
        std::string()};
    fcitx::Option<std::string> llmApiUrl{
        this,
        "LlmApiUrl",
        "LLM API Base URL",
        std::string("https://open.bigmodel.cn/api/paas/v4")};
    fcitx::Option<std::string> llmModel{
        this,
        "LlmModel",
        "LLM Model Name",
        std::string("glm-4.7-flash")};
    fcitx::Option<int> llmTimeout{
        this,
        "LlmTimeout",
        "LLM Timeout (seconds, 5-120)",
        15};
    fcitx::Option<bool> llmThinkingMode{
        this,
        "LlmThinkingMode",
        "Enable LLM Thinking Mode (for reasoning models)",
        false};
    fcitx::Option<int> llmMaxTokens{
        this,
        "LlmMaxTokens",
        "LLM Max Output Tokens (prevent loops, 256-8192)",
        2048};
    fcitx::Option<bool> resetLlmPrompts{
        this,
        "ResetLlmPrompts",
        "Reset LLM Prompts to Default (\xe2\x98\x91 check and apply, auto-unchecks after reset. Edit: ~/.config/glm-asrd/prompts/)",
        false};
);

class GlmAsrAddon : public fcitx::AddonInstance {
public:
    GlmAsrAddon(fcitx::Instance *instance);
    ~GlmAsrAddon();

    void reloadConfig() override;
    const fcitx::Configuration *getConfig() const override;
    void setConfig(const fcitx::RawConfig &config) override;

    void candidateSelected(int index);

private:
    enum class State { Idle, Armed, Recording, Processing, ResultReady, Selecting };

    struct CandidateEntry {
        std::string text;
        std::string source;
        float confidence = 0.0f;
    };

    struct CandidateSession {
        std::vector<CandidateEntry> entries;
        std::string asrRawText;
        int cursorIndex = 0;
    };

    void handleKeyEvent(fcitx::KeyEvent &keyEvent);
    void handleFocusOut(fcitx::Event &event);

    void showStatus(const std::string &text);
    void clearStatus();
    void commitText(const std::string &text);

    int connectToDaemon();
    void cleanupIO(std::unique_ptr<fcitx::EventSourceIO> &io, int &fd);
    bool sendConfigToDaemon();

    void onArmTimer(fcitx::EventSourceTime *src, uint64_t);
    void onRecordIO(fcitx::EventSourceIO *src, int fd, fcitx::IOEventFlags flags);
    void onResultIO(fcitx::EventSourceIO *src, int fd, fcitx::IOEventFlags flags);
    void onDisplayTimer(fcitx::EventSourceTime *src, uint64_t);

    void handleSelectionKey(fcitx::KeyEvent &keyEvent);
    void showCandidates();
    void updateCandidateDisplay();
    void commitSelected();
    void cancelSelection();

    static std::string parseJsonField(const std::string &json, const std::string &key);
    static std::string escapeJson(const std::string &s);
    static std::vector<CandidateEntry> parseCandidates(const std::string &json);
    static std::string formatCandidateDisplay(const CandidateEntry &entry);

    void startRecording(fcitx::InputContext *ic);
    void stopRecording();

    fcitx::Instance *instance_;
    GlmAsrConfig config_;
    State state_ = State::Idle;
    fcitx::InputContext *currentIc_ = nullptr;

    std::vector<std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>>> handlers_;

    std::unique_ptr<fcitx::EventSourceTime> armTimer_;
    std::unique_ptr<fcitx::EventSourceIO> recordIO_;
    std::string recordBuf_;
    int recordFd_ = -1;

    std::unique_ptr<fcitx::EventSourceIO> resultIO_;
    std::string resultBuf_;
    int resultFd_ = -1;

    std::unique_ptr<fcitx::EventSourceTime> displayTimer_;
    std::string pendingResult_;
    fcitx::InputContext *resultIc_ = nullptr;

    CandidateSession session_;
};
