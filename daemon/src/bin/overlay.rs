use tracing::{error, info};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new("glm_asrd=info")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut renderer_type = glm_asrd::overlay::OverlayRendererType::Software;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--renderer" => {
                if let Some(val) = args.next() {
                    renderer_type = if val.starts_with("Vello") {
                        glm_asrd::overlay::OverlayRendererType::Vello
                    } else {
                        glm_asrd::overlay::OverlayRendererType::Software
                    };
                }
            }
            _ => {}
        }
    }

    info!("glm-asr-overlay starting (renderer={:?})", renderer_type);

    match glm_asrd::overlay::run_overlay_stdio(renderer_type) {
        Ok(()) => info!("glm-asr-overlay exited cleanly"),
        Err(e) => error!("glm-asr-overlay error: {e}"),
    }
}
