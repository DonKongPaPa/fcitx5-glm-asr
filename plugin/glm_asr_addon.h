#pragma once

#include <fcitx/addoninstance.h>
#include <fcitx-config/configuration.h>
#include <fcitx-config/option.h>
#include <fcitx-config/iniparser.h>
#include <fcitx-config/rawconfig.h>
#include <fcitx-utils/handlertable.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx/instance.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/event.h>

#include <memory>
#include <string>
#include <vector>

FCITX_CONFIGURATION(
    GlmAsrConfig,
    fcitx::Option<std::string> apiKey{
        this,
        "ApiKey",
        "API Key",
        std::string()};
    fcitx::Option<std::string> model{
        this,
        "Model",
        "ASR Model",
        std::string("glm-asr-2512")};
    fcitx::Option<std::string> apiUrl{
        this,
        "ApiUrl",
        "API URL",
        std::string("https://open.bigmodel.cn/api/paas/v4/audio/transcriptions")};
    fcitx::Option<int> sampleRate{
        this,
        "SampleRate",
        "Sample Rate (Hz)",
        16000};
);

class GlmAsrAddon : public fcitx::AddonInstance {
public:
    GlmAsrAddon(fcitx::Instance *instance);
    ~GlmAsrAddon();

    void reloadConfig() override;
    const fcitx::Configuration *getConfig() const override;
    void setConfig(const fcitx::RawConfig &config) override;

private:
    enum class State { Idle, Armed, Recording, Processing, ResultReady };

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

    static std::string parseJsonField(const std::string &json, const std::string &key);
    static std::string escapeJson(const std::string &s);

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
};
