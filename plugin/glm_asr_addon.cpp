#include "glm_asr_addon.h"

#include <fcitx/addonfactory.h>
#include <fcitx/addonmanager.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx-utils/log.h>
#include <fcitx-utils/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/event.h>
#include <fcitx/candidatelist.h>

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <cstring>
#include <ctime>
#include <cstdlib>

static const char *CONFIG_FILE = "conf/glm-asr.conf";
static const uint64_t ARM_THRESHOLD_USEC = 300000;
static const uint64_t DISPLAY_DELAY_USEC = 400000;

class AsrCandidateWord : public fcitx::CandidateWord {
public:
    AsrCandidateWord(fcitx::Text text, int index, GlmAsrAddon *addon)
        : CandidateWord(std::move(text)), index_(index), addon_(addon) {}

    void select(fcitx::InputContext *) const override {
        addon_->candidateSelected(index_);
    }

private:
    int index_;
    GlmAsrAddon *addon_;
};

GlmAsrAddon::GlmAsrAddon(fcitx::Instance *instance) : instance_(instance) {
    reloadConfig();

    handlers_.emplace_back(instance_->watchEvent(
        fcitx::EventType::InputContextKeyEvent,
        fcitx::EventWatcherPhase::PreInputMethod,
        [this](fcitx::Event &event) {
            auto &keyEvent = static_cast<fcitx::KeyEvent &>(event);
            handleKeyEvent(keyEvent);
        }));

    handlers_.emplace_back(instance_->watchEvent(
        fcitx::EventType::InputContextFocusOut,
        fcitx::EventWatcherPhase::Default,
        [this](fcitx::Event &event) {
            handleFocusOut(event);
        }));

    sendConfigToDaemon();

    FCITX_INFO() << "glm-asr addon loaded";
}

GlmAsrAddon::~GlmAsrAddon() {
    cleanupIO(recordIO_, recordFd_);
    cleanupIO(resultIO_, resultFd_);
}

void GlmAsrAddon::reloadConfig() {
    fcitx::readAsIni(config_, CONFIG_FILE);
}

const fcitx::Configuration *GlmAsrAddon::getConfig() const {
    return &config_;
}

void GlmAsrAddon::setConfig(const fcitx::RawConfig &config) {
    config_.load(config, true);
    fcitx::safeSaveAsIni(config_, CONFIG_FILE);
    sendConfigToDaemon();
    if (config_.resetLlmPrompts.value()) {
        config_.resetLlmPrompts.setValue(false);
        fcitx::safeSaveAsIni(config_, CONFIG_FILE);
    }
}

std::string GlmAsrAddon::escapeJson(const std::string &s) {
    std::string out;
    out.reserve(s.size() + 4);
    for (char c : s) {
        switch (c) {
        case '"':  out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case '\n': out += "\\n"; break;
        case '\r': out += "\\r"; break;
        case '\t': out += "\\t"; break;
        default:   out += c; break;
        }
    }
    return out;
}

bool GlmAsrAddon::sendConfigToDaemon() {
    int fd = connectToDaemon();
    if (fd < 0) {
        FCITX_INFO() << "glm-asr: daemon not running, will sync config on next connection";
        return false;
    }

    std::string cmd = "{\"cmd\":\"set_config\","
        "\"api_key\":\"" + escapeJson(config_.apiKey.value()) + "\","
        "\"model\":\"" + escapeJson(config_.model.value()) + "\","
        "\"api_url\":\"" + escapeJson(config_.apiUrl.value()) + "\","
        "\"sample_rate\":" + SampleRateToString(config_.sampleRate.value()) + ","
        "\"use_overlay\":" + (config_.useOverlay.value() ? "true" : "false") + ","
        "\"overlay_renderer\":\"" + OverlayRendererToString(config_.overlayRenderer.value()) + "\","
        "\"enable_llm\":" + (config_.enableLlm.value() ? "true" : "false") + ","
        "\"llm_api_key\":\"" + escapeJson(config_.llmApiKey.value()) + "\","
        "\"llm_api_url\":\"" + escapeJson(config_.llmApiUrl.value()) + "\","
        "\"llm_model\":\"" + escapeJson(config_.llmModel.value()) + "\","
        "\"llm_timeout_secs\":" + std::to_string(std::max(5, std::min(120, config_.llmTimeout.value()))) + ","
        "\"llm_thinking_mode\":" + (config_.llmThinkingMode.value() ? "true" : "false") + ","
        "\"llm_max_tokens\":" + std::to_string(std::max(256, std::min(8192, config_.llmMaxTokens.value()))) + ","
        "\"reset_llm_prompts\":" + (config_.resetLlmPrompts.value() ? "true" : "false")
    + "}\n";

    ssize_t w = write(fd, cmd.c_str(), cmd.size());
    ::close(fd);

    if (w < 0 || static_cast<size_t>(w) != cmd.size()) {
        FCITX_INFO() << "glm-asr: failed to send config to daemon";
        return false;
    }

    FCITX_INFO() << "glm-asr: config synced to daemon";
    return true;
}

void GlmAsrAddon::handleFocusOut(fcitx::Event &event) {
    auto *ic = static_cast<fcitx::InputContextEvent &>(event).inputContext();
    if (ic == currentIc_) {
        if (state_ == State::Recording || state_ == State::Armed) {
            armTimer_.reset();
            cleanupIO(recordIO_, recordFd_);
            state_ = State::Idle;
            currentIc_ = nullptr;
        } else if (state_ == State::Selecting) {
            cancelSelection();
        }
    }
}

int GlmAsrAddon::connectToDaemon() {
    std::string path = "/run/user/" + std::to_string(getuid()) + "/glm-asrd.sock";
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0);
    if (fd < 0) return -1;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);

    if (::connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        if (errno != EINPROGRESS) {
            ::close(fd);
            return -1;
        }
    }
    return fd;
}

void GlmAsrAddon::cleanupIO(std::unique_ptr<fcitx::EventSourceIO> &io, int &fd) {
    io.reset();
    if (fd >= 0) {
        ::close(fd);
        fd = -1;
    }
}

void GlmAsrAddon::handleKeyEvent(fcitx::KeyEvent &keyEvent) {
    auto *ic = keyEvent.inputContext();
    if (!ic) return;

    if (state_ == State::Selecting) {
        handleSelectionKey(keyEvent);
        return;
    }

    if (!keyEvent.key().checkKeyList(config_.triggerKey.value())) {
        return;
    }

    keyEvent.filterAndAccept();

    TriggerMode mode = config_.triggerMode.value();

    if (mode == TriggerMode::Hold) {
        if (!keyEvent.isRelease() && state_ == State::Idle) {
            state_ = State::Armed;
            currentIc_ = ic;

            uint64_t deadline = fcitx::now(CLOCK_MONOTONIC) + ARM_THRESHOLD_USEC;
            armTimer_ = instance_->eventLoop().addTimeEvent(
                CLOCK_MONOTONIC, deadline, 0,
                [this](fcitx::EventSourceTime *src, uint64_t) {
                    onArmTimer(src, 0);
                    return true;
                });
            return;
        }

        if (keyEvent.isRelease()) {
            armTimer_.reset();

            if (state_ == State::Armed) {
                state_ = State::Idle;
                currentIc_ = nullptr;
                return;
            }

            if (state_ == State::Recording) {
                stopRecording();
            }
        }
    } else {
        if (keyEvent.isRelease()) return;

        if (state_ == State::Idle) {
            currentIc_ = ic;
            startRecording(ic);
        } else if (state_ == State::Recording) {
            stopRecording();
        }
    }
}

void GlmAsrAddon::startRecording(fcitx::InputContext *ic) {
    state_ = State::Recording;
    currentIc_ = ic;

    if (!config_.useOverlay.value()) {
        showStatus("\xf0\x9f\x8e\x99\xef\xb8\x8f \xe6\xad\xa3\xe5\x9c\xa8\xe5\xbd\x95\xe9\x9f\xb3...");
    }

    int fd = connectToDaemon();
    if (fd < 0) {
        showStatus("\xe2\x9d\x8c \xe6\x97\xa0\xe6\xb3\x95\xe8\xbf\x9e\xe6\x8e\xa5\xe5\xae\x88\xe6\x8a\xa4\xe8\xbf\x9b\xe7\xa8\x8b");
        state_ = State::Idle;
        currentIc_ = nullptr;
        return;
    }

    std::string cmd = "{\"cmd\":\"start_record\",\"use_overlay\":"
        + std::string(config_.useOverlay.value() ? "true" : "false") + "}\n";
    ssize_t w = write(fd, cmd.c_str(), cmd.size());
    if (w < 0 || static_cast<size_t>(w) != cmd.size()) {
        ::close(fd);
        showStatus("\xe2\x9d\x8c \xe5\x8f\x91\xe9\x80\x81\xe5\xa4\xb1\xe8\xb4\xa5");
        state_ = State::Idle;
        currentIc_ = nullptr;
        return;
    }

    recordFd_ = fd;
    recordBuf_.clear();

    recordIO_ = instance_->eventLoop().addIOEvent(
        fd, fcitx::IOEventFlag::In,
        [this](fcitx::EventSourceIO *src, int fd, fcitx::IOEventFlags flags) {
            onRecordIO(src, fd, flags);
            return true;
        });
}

void GlmAsrAddon::stopRecording() {
    if (state_ != State::Recording) return;

    state_ = State::Processing;

    if (!config_.useOverlay.value()) {
        showStatus("\xe2\x8f\xb3 \xe8\xaf\x86\xe5\x88\xab\xe4\xb8\xad...");
    }

    cleanupIO(recordIO_, recordFd_);

    int fd = connectToDaemon();
    if (fd < 0) {
        showStatus("\xe2\x9d\x8c \xe6\x97\xa0\xe6\xb3\x95\xe8\xbf\x9e\xe6\x8e\xa5\xe5\xae\x88\xe6\x8a\xa4\xe8\xbf\x9b\xe7\xa8\x8b");
        state_ = State::Idle;
        currentIc_ = nullptr;
        return;
    }

    std::string cmd = "{\"cmd\":\"stop_record\",\"use_overlay\":"
        + std::string(config_.useOverlay.value() ? "true" : "false") + "}\n";
    ssize_t w = write(fd, cmd.c_str(), cmd.size());
    if (w < 0 || static_cast<size_t>(w) != cmd.size()) {
        ::close(fd);
        showStatus("\xe2\x9d\x8c \xe5\x8f\x91\xe9\x80\x81\xe5\xa4\xb1\xe8\xb4\xa5");
        state_ = State::Idle;
        currentIc_ = nullptr;
        return;
    }

    resultFd_ = fd;
    resultBuf_.clear();
    resultIc_ = currentIc_;

    resultIO_ = instance_->eventLoop().addIOEvent(
        fd, fcitx::IOEventFlag::In,
        [this](fcitx::EventSourceIO *src, int fd, fcitx::IOEventFlags flags) {
            onResultIO(src, fd, flags);
            return true;
        });
}

void GlmAsrAddon::onArmTimer(fcitx::EventSourceTime *, uint64_t) {
    if (state_ != State::Armed) return;

    startRecording(currentIc_);
}

void GlmAsrAddon::onRecordIO(fcitx::EventSourceIO *, int fd, fcitx::IOEventFlags flags) {
    if ((flags & fcitx::IOEventFlag::Err) || (flags & fcitx::IOEventFlag::Hup)) {
        cleanupIO(recordIO_, recordFd_);
        return;
    }

    char buf[4096];
    ssize_t n = read(fd, buf, sizeof(buf));
    if (n <= 0) {
        cleanupIO(recordIO_, recordFd_);
        return;
    }
    recordBuf_.append(buf, n);
}

void GlmAsrAddon::onResultIO(fcitx::EventSourceIO *, int fd, fcitx::IOEventFlags flags) {
    if ((flags & fcitx::IOEventFlag::Err) || (flags & fcitx::IOEventFlag::Hup)) {
        if (resultBuf_.empty()) {
            showStatus("\xe2\x9d\x8c \xe8\xbf\x9e\xe6\x8e\xa5\xe6\x96\xad\xe5\xbc\x80");
        }
        cleanupIO(resultIO_, resultFd_);
        state_ = State::Idle;
        return;
    }

    char buf[4096];
    ssize_t n = read(fd, buf, sizeof(buf));
    if (n < 0) {
        if (errno == EAGAIN || errno == EWOULDBLOCK) return;
        cleanupIO(resultIO_, resultFd_);
        state_ = State::Idle;
        return;
    }
    if (n == 0) {
        cleanupIO(resultIO_, resultFd_);
        state_ = State::Idle;
        return;
    }
    resultBuf_.append(buf, n);

    size_t pos;
    while ((pos = resultBuf_.find('\n')) != std::string::npos) {
        std::string line = resultBuf_.substr(0, pos);
        resultBuf_.erase(0, pos + 1);
        if (line.empty()) continue;

        std::string type = parseJsonField(line, "type");
        if (type == "result") {
            std::string text = parseJsonField(line, "text");
            cleanupIO(resultIO_, resultFd_);

            auto candidates = parseCandidates(line);

            std::string asrRaw;
            for (const auto &c : candidates) {
                if (c.source == "asr") {
                    asrRaw = c.text;
                    break;
                }
            }
            if (asrRaw.empty()) {
                asrRaw = text;
            }

            if (candidates.size() > 1) {
                session_.entries = std::move(candidates);
                session_.asrRawText = asrRaw;
                session_.cursorIndex = 0;
                state_ = State::Selecting;
                resultIc_ = currentIc_;
                showCandidates();
                return;
            }

            if (asrRaw.empty() || !resultIc_) {
                clearStatus();
                state_ = State::Idle;
                currentIc_ = nullptr;
                resultIc_ = nullptr;
                return;
            }

            if (config_.useOverlay.value()) {
                state_ = State::Idle;
                commitText(asrRaw);
                clearStatus();
                currentIc_ = nullptr;
                resultIc_ = nullptr;
                return;
            }

            state_ = State::ResultReady;
            pendingResult_ = asrRaw;

            std::string display = "\xe2\x9c\x85 ";
            if (asrRaw.size() > 20) {
                display += asrRaw.substr(0, 20) + "...";
            } else {
                display += asrRaw;
            }
            showStatus(display);

            uint64_t deadline = fcitx::now(CLOCK_MONOTONIC) + DISPLAY_DELAY_USEC;
            displayTimer_ = instance_->eventLoop().addTimeEvent(
                CLOCK_MONOTONIC, deadline, 0,
                [this](fcitx::EventSourceTime *, uint64_t) {
                    onDisplayTimer(nullptr, 0);
                    return true;
                });
            return;
        }
        if (type == "error") {
            std::string msg = parseJsonField(line, "message");
            if (!config_.useOverlay.value()) {
                showStatus("\xe2\x9d\x8c " + msg);
            }
            cleanupIO(resultIO_, resultFd_);
            state_ = State::Idle;
            currentIc_ = nullptr;
            resultIc_ = nullptr;
            return;
        }
    }
}

void GlmAsrAddon::onDisplayTimer(fcitx::EventSourceTime *, uint64_t) {
    if (state_ == State::ResultReady && resultIc_) {
        commitText(pendingResult_);
    }
    clearStatus();
    state_ = State::Idle;
    currentIc_ = nullptr;
    resultIc_ = nullptr;
    pendingResult_.clear();
    displayTimer_.reset();
}

void GlmAsrAddon::handleSelectionKey(fcitx::KeyEvent &keyEvent) {
    if (keyEvent.isRelease()) return;

    const auto &key = keyEvent.key();
    int total = static_cast<int>(session_.entries.size());

    if (key.check(fcitx::Key(FcitxKey_Tab)) ||
        key.check(fcitx::Key(FcitxKey_Down))) {
        keyEvent.filterAndAccept();
        session_.cursorIndex = (session_.cursorIndex + 1) % total;
        updateCandidateDisplay();
        return;
    }

    if (key.check(fcitx::Key(FcitxKey_Tab, fcitx::KeyState::Shift)) ||
        key.check(fcitx::Key(FcitxKey_Up))) {
        keyEvent.filterAndAccept();
        session_.cursorIndex = (session_.cursorIndex - 1 + total) % total;
        updateCandidateDisplay();
        return;
    }

    if (key.check(fcitx::Key(FcitxKey_Return)) ||
        key.check(fcitx::Key(FcitxKey_space))) {
        keyEvent.filterAndAccept();
        commitSelected();
        return;
    }

    if (key.check(fcitx::Key(FcitxKey_Escape))) {
        keyEvent.filterAndAccept();
        cancelSelection();
        return;
    }

    if (key.sym() >= FcitxKey_1 && key.sym() <= FcitxKey_9) {
        int idx = key.sym() - FcitxKey_1;
        if (idx < total) {
            keyEvent.filterAndAccept();
            session_.cursorIndex = idx;
            commitSelected();
            return;
        }
    }
}

void GlmAsrAddon::showCandidates() {
    if (!currentIc_) return;

    auto &panel = currentIc_->inputPanel();
    panel.reset();

    const auto &entry = session_.entries[session_.cursorIndex];
    std::string preview = entry.text.empty() ? session_.asrRawText : entry.text;

    if (config_.usePreedit.value()) {
        panel.setPreedit(fcitx::Text(preview));
    } else {
        std::string hint = "\xf0\x9f\x8e\xa4 ";
        hint += std::to_string(session_.entries.size());
        hint += " \xe4\xb8\xaa\xe5\x80\x99\xe9\x80\x89";
        panel.setAuxUp(fcitx::Text(hint));
    }

    auto candList = std::make_unique<fcitx::CommonCandidateList>();
    candList->setPageSize(9);
    candList->setLayoutHint(fcitx::CandidateLayoutHint::Vertical);
    candList->setSelectionKey({fcitx::Key("1"), fcitx::Key("2"), fcitx::Key("3"),
                               fcitx::Key("4"), fcitx::Key("5"), fcitx::Key("6"),
                               fcitx::Key("7"), fcitx::Key("8"), fcitx::Key("9")});

    int idx = 0;
    for (const auto &e : session_.entries) {
        std::string display = formatCandidateDisplay(e);
        candList->append(std::make_unique<AsrCandidateWord>(fcitx::Text(display), idx++, this));
    }
    candList->setGlobalCursorIndex(session_.cursorIndex);

    panel.setCandidateList(std::move(candList));
    currentIc_->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);

    if (config_.usePreedit.value()) {
        currentIc_->updatePreedit();
    }
}

void GlmAsrAddon::updateCandidateDisplay() {
    showCandidates();
}

void GlmAsrAddon::commitSelected() {
    if (session_.cursorIndex < static_cast<int>(session_.entries.size()) && resultIc_) {
        const auto &entry = session_.entries[session_.cursorIndex];
        bool is_error = entry.source.find("error_") == 0;
        if (is_error || entry.source == "asr") {
            resultIc_->commitString(session_.asrRawText);
        } else {
            resultIc_->commitString(entry.text);
        }
    }
    clearStatus();
    state_ = State::Idle;
    currentIc_ = nullptr;
    resultIc_ = nullptr;
    session_.entries.clear();
    session_.asrRawText.clear();
    session_.cursorIndex = 0;
}

void GlmAsrAddon::cancelSelection() {
    clearStatus();
    state_ = State::Idle;
    currentIc_ = nullptr;
    resultIc_ = nullptr;
    session_.entries.clear();
    session_.asrRawText.clear();
    session_.cursorIndex = 0;
}

void GlmAsrAddon::candidateSelected(int index) {
    if (state_ != State::Selecting) return;
    if (index < 0 || index >= static_cast<int>(session_.entries.size())) return;
    session_.cursorIndex = index;
    commitSelected();
}

void GlmAsrAddon::showStatus(const std::string &text) {
    if (!currentIc_) return;
    auto &panel = currentIc_->inputPanel();
    panel.reset();
    panel.setAuxUp(fcitx::Text(text));
    currentIc_->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

void GlmAsrAddon::clearStatus() {
    if (!currentIc_) return;
    auto &panel = currentIc_->inputPanel();
    panel.reset();
    currentIc_->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

void GlmAsrAddon::commitText(const std::string &text) {
    if (!resultIc_) return;
    resultIc_->commitString(text);
}

std::string GlmAsrAddon::parseJsonField(const std::string &json, const std::string &key) {
    std::string search = "\"" + key + "\":";
    auto pos = json.find(search);
    if (pos == std::string::npos) return "";
    pos += search.size();
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) pos++;
    if (pos < json.size() && json[pos] == '"') {
        pos++;
        auto end = json.find('"', pos);
        if (end == std::string::npos) return "";
        return json.substr(pos, end - pos);
    }
    if (pos < json.size() && (json[pos] == 't' || json[pos] == 'f')) {
        return json[pos] == 't' ? "true" : "false";
    }
    return "";
}

std::string GlmAsrAddon::formatCandidateDisplay(const CandidateEntry &entry) {
    if (entry.source == "asr") {
        return "[\xe5\x8e\x9f\xe5\xa7\x8b] " + entry.text;
    }
    if (entry.source.find("error_") == 0) {
        std::string label = entry.source.substr(6);
        return "[\xe2\x9d\x8c " + label + "]";
    }
    if (entry.source.find("llm_") == 0) {
        std::string label = entry.source.substr(4);
        return "[\xe2\x9c\x85 " + label + "] " + entry.text;
    }
    return entry.text;
}

std::vector<GlmAsrAddon::CandidateEntry> GlmAsrAddon::parseCandidates(const std::string &json) {
    std::vector<CandidateEntry> result;
    auto key_pos = json.find("\"candidates\":");
    if (key_pos == std::string::npos) return result;

    auto arr_start = json.find('[', key_pos);
    if (arr_start == std::string::npos) return result;

    size_t pos = arr_start + 1;
    while (pos < json.size()) {
        auto obj_end = json.find(']', pos);
        auto obj_pos = json.find("\"text\":", pos);
        if (obj_pos == std::string::npos || (obj_end != std::string::npos && obj_pos > obj_end)) break;

        auto quote_start = json.find('"', obj_pos + 7);
        if (quote_start == std::string::npos) break;
        quote_start++;
        auto quote_end = json.find('"', quote_start);
        if (quote_end == std::string::npos) break;

        std::string text = json.substr(quote_start, quote_end - quote_start);
        std::string source;

        auto src_pos = json.find("\"source\":", quote_end);
        auto next_obj = json.find("\"text\":", quote_end);
        if (src_pos != std::string::npos && (next_obj == std::string::npos || src_pos < next_obj)) {
            auto src_quote = json.find('"', src_pos + 9);
            if (src_quote != std::string::npos) {
                src_quote++;
                auto src_end = json.find('"', src_quote);
                if (src_end != std::string::npos) {
                    source = json.substr(src_quote, src_end - src_quote);
                }
            }
        }

        float conf = 0.0f;
        auto conf_pos = json.find("\"confidence\":", quote_end);
        if (conf_pos != std::string::npos && (next_obj == std::string::npos || conf_pos < next_obj)) {
            auto val_start = conf_pos + 13;
            while (val_start < json.size() && json[val_start] == ' ') val_start++;
            auto val_end = json.find_first_of(",}", val_start);
            if (val_end != std::string::npos) {
                try {
                    conf = std::stof(json.substr(val_start, val_end - val_start));
                } catch (...) {}
            }
        }

        result.push_back({text, source, conf});
        pos = quote_end + 1;
    }
    return result;
}

class GlmAsrAddonFactory : public fcitx::AddonFactory {
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
        return new GlmAsrAddon(manager->instance());
    }
};

FCITX_ADDON_FACTORY(GlmAsrAddonFactory);
