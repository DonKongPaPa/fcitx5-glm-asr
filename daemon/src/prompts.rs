use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

const FRAMEWORK_SUFFIX: &str = "\

根据以上指导，分析原文、推断使用场景、检测用户意图，然后输出你认为合适的纠正版本。

严格按JSON格式返回结果：
{\"candidates\":[{\"text\":\"纠正后文本\",\"label\":\"标签\",\"confidence\":0.95}]}

规则：
- confidence 为 0.0-1.0 的浮点数，表示该纠正的适用置信度
- label 必须使用中文描述纠正类型（如\"通顺版\"、\"规范版\"、\"日期格式\"、\"英语翻译\"等）
- 通顺版和规范版应始终返回（除非原文已完美无缺），其他纠正按需返回
- 显式指令产生的候选项 confidence 应较高（0.9+）
- 不要无意义重复
- 不要添加原文中没有的信息
- 按 confidence 降序排列";

pub fn prompts_dir() -> PathBuf {
    let config_dir = super::config::Config::config_dir();
    config_dir.join("prompts")
}

pub fn ensure_prompts_dir() -> PathBuf {
    let dir = prompts_dir();
    let advices_dir = dir.join("correction_advices");
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
        info!("Created prompts directory: {:?}", dir);
    }
    if !advices_dir.exists() {
        let _ = fs::create_dir_all(&advices_dir);
        info!("Created correction_advices directory: {:?}", advices_dir);
    }
    dir
}

pub fn load_main_prompt() -> Result<String, String> {
    let path = prompts_dir().join("main.md");
    if !path.exists() {
        generate_default_main_md();
    }
    fs::read_to_string(&path)
        .map(|c| {
            let content = c.trim().to_string();
            info!("Loaded main prompt ({}chars)", content.len());
            content
        })
        .map_err(|e| format!("Failed to read main.md: {}", e))
}

pub fn load_correction_advices() -> Result<Vec<(String, String)>, String> {
    let dir = prompts_dir().join("correction_advices");
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    ensure_default_advices(&dir);

    let mut advices = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("Failed to read correction_advices dir: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let filename = path.file_stem().unwrap().to_string_lossy().to_string();
            match fs::read_to_string(&path) {
                Ok(content) => {
                    let trimmed = content.trim().to_string();
                    if !trimmed.is_empty() {
                        info!("Loaded correction advice: {} ({}chars)", filename, trimmed.len());
                        advices.push((filename, trimmed));
                    }
                }
                Err(e) => {
                    warn!("Failed to read advice file {:?}: {}", path, e);
                }
            }
        }
    }

    if advices.is_empty() {
        return Err("No correction advice files found. At least one .md file in correction_advices/ is required.".to_string());
    }

    advices.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(advices)
}

pub fn merge_system_prompt() -> Result<String, String> {
    let main = load_main_prompt()?;
    let advices = load_correction_advices()?;

    let mut system = main;
    system.push_str("\n\n---\n\n以下是纠正指导，根据原文内容和场景推断按需应用：\n");

    for (name, content) in &advices {
        let display_name = name.replace('_', " ");
        system.push_str(&format!("\n【{}】\n{}\n", display_name, content));
    }

    system.push_str(FRAMEWORK_SUFFIX);
    Ok(system)
}

pub fn reset_all_prompts() -> Result<(), String> {
    let dir = prompts_dir();
    if dir.exists() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("Failed to read dir: {}", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                let _ = fs::remove_file(&path);
            }
        }
        let advices_dir = dir.join("correction_advices");
        if advices_dir.exists() {
            let entries = fs::read_dir(&advices_dir).map_err(|e| format!("Failed to read dir: {}", e))?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    ensure_prompts_dir();
    generate_default_main_md();
    generate_default_advices();
    info!("All prompts reset to defaults");
    Ok(())
}

fn generate_default_main_md() {
    let path = prompts_dir().join("main.md");
    if path.exists() {
        return;
    }
    let content = "\
## Main

你是一个语音识别(ASR)结果后处理助手。用户将提供一段语音识别的原始文本。

你的任务是分析原文中可能存在的识别错误，并根据纠正指导生成不同风格的纠正版本。

核心原则：
- 保持原文语义和意图不变
- 保留原文中的英文单词和短语，支持中英文混合表达
- 首先确保原文读起来通顺，修正明显的识别错误
- 每个纠正版本必须是有意义的变换
- 每个版本附带 confidence（0.0-1.0）表示适用程度

纠正强度：
所有场景都应返回两个基础纠正版本：
- 通顺版（轻度纠正）：只修正明显的识别错误（同音字、近音字），确保语句通顺可读。
  不改变用词、不补充标点、不去除语气词、不调整语序。confidence 应最高。
- 规范版（深度纠正）：在通顺版基础上，优化用词、补充标点、调整语序，
  使表达更规范正式。适用于正式文档场景。confidence 视场景而定。

场景推断：
首先确保原文通顺可读，然后推断使用场景：
- 社交聊天：即时通讯场景，口语化表达完全正常。通顺版 confidence 最高，
  规范版 confidence 较低。
- 正式文档：邮件、报告等。通顺版和规范版 confidence 都应较高。
- 数据输入：表格、表单等，数字和日期格式化更重要。
- 搜索查询：关键词形式更合适，去除冗余词。

意图识别：
同时检测原文中是否包含用户指令（而非待处理内容）：
- 翻译指令：如\"翻译成英语\"、\"用英文说\"、\"translate\"等。识别后去除指令部分，
  对剩余内容执行翻译。原文纠错版本仍应保留。
- 数字转换指令：如\"大写数字\"、\"转成大写\"、\"小写转大写\"等。
  对原文中的数字执行对应转换。
- 其他显式指令：如果用户明确表达了某种转换需求，优先执行。

场景推断和意图识别会影响纠正建议的适用度（confidence），请在 confidence 中体现。
显式指令产生的候选项 confidence 应较高（0.9+）。
";
    match fs::write(&path, content) {
        Ok(_) => info!("Generated default main.md"),
        Err(e) => warn!("Failed to write main.md: {}", e),
    }
}

fn ensure_default_advices(dir: &PathBuf) {
    let count = fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("md")).count())
        .unwrap_or(0);
    if count == 0 {
        generate_default_advices();
    }
}

fn generate_default_advices() {
    let dir = prompts_dir().join("correction_advices");
    let _ = fs::create_dir_all(&dir);

    let defaults = [
        ("basic_correction.md", "轻度纠正（通顺版）：\n只修正语音识别中明显的同音字、近音字错误，确保句子读起来通顺。\n不改变原文的用词、语气、风格。保留英文单词、缩写、网络用语。\n不补充标点（除非明显缺失导致歧义），不去除语气词。\n\n深度纠正（规范版）：\n在通顺版基础上，进一步优化用词，补充标点，调整语序，\n使表达更加规范、正式。但不要过度纠正，不要改变原文的语义和意图。"),
        ("date_number_format.md", "当原文包含日期或数字时，将其转换为标准格式。例如：\n- 口述日期 \"二零二五年五月八号\" → \"2025年5月8日\"\n- 连续数字 \"一万二千三百\" → \"12,300\"\n- 口述电话号码 → 按常见格式分段显示\n如果不包含相关内容，跳过此纠正。"),
        ("spoken_to_written.md", "将口语化表达转换为正式的书面语。去除语气词（\"嗯\"、\"那个\"、\"就是说\"等）、重复词、口头禅，调整句式结构使其更规范。如果原文已经是书面风格，跳过此纠正。"),
        ("translation.md", "检测原文中的翻译指令（如\"翻译成英语\"、\"用英文说\"、\"翻译为日语\"等）。如果检测到翻译意图：\n1. 从原文中去除指令部分，提取待翻译的实际内容\n2. 将内容翻译为目标语言\n3. 将翻译结果作为一个高 confidence 候选\n4. 同时保留原始内容的纠错版本\n\n支持常见语言：英语、日语、韩语、法语、德语等。\n如果未检测到翻译意图，跳过此纠正。"),
        ("number_conversion.md", "检测原文中的数字转换指令或场景：\n- 大写转换：如用户说\"大写数字\"或处于财务/合同场景，将阿拉伯数字转为中文大写（如 1,000,000 → 壹佰万，123 → 壹佰贰拾叁）\n- 货币格式：如包含金额，转换为标准货币格式（如\"一百块\" → \"¥100.00\"）\n- 指令驱动：如用户说\"转成大写\"，对原文中所有数字执行转换\n\n中文大写数字对照：零壹贰叁肆伍陆柒捌玖 拾佰仟万亿\n如果原文不包含数字或无转换需求，跳过此纠正。"),
    ];

    for (filename, content) in &defaults {
        let path = dir.join(filename);
        if !path.exists() {
            match fs::write(&path, content) {
                Ok(_) => info!("Generated default advice: {}", filename),
                Err(e) => warn!("Failed to write {}: {}", filename, e),
            }
        }
    }
}

pub fn cleanup_orphan_overlays() {
    let current_pid = std::process::id();
    let output = match std::process::Command::new("pgrep")
        .args(["-a", "glm-asr-overlay"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            continue;
        }
        if let Ok(pid) = parts[0].parse::<u32>() {
            let ppid_output = match std::process::Command::new("ps")
                .args(["-o", "ppid=", "-p", &pid.to_string()])
                .output()
            {
                Ok(o) => o,
                Err(_) => continue,
            };
            let ppid_str = String::from_utf8_lossy(&ppid_output.stdout).trim().to_string();
            if let Ok(ppid) = ppid_str.parse::<u32>() {
                if ppid != current_pid {
                    info!("Killing orphan overlay process (pid={}, ppid={})", pid, ppid);
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }
        }
    }
}
