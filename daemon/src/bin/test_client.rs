use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn main() {
    let socket = std::env::var("GLM_ASRD_SOCK")
        .unwrap_or_else(|_| format!("/run/user/{}/glm-asrd.sock", unsafe { libc::getuid() }));

    let cmd = std::env::args()
        .nth(1)
        .expect("Usage: test_client <start_record|stop_record|ping>");

    let msg = match cmd.as_str() {
        "start" => serde_json::json!({"cmd": "start_record"}),
        "stop" => serde_json::json!({"cmd": "stop_record"}),
        "ping" => serde_json::json!({"cmd": "ping"}),
        _ => panic!("Unknown command: {}", cmd),
    };

    let mut stream = UnixStream::connect(&socket).expect("Failed to connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();

    writeln!(stream, "{}", msg).unwrap();
    stream.flush().unwrap();

    let reader = std::io::BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(l) => {
                println!("{}", l);
                let resp: serde_json::Value = serde_json::from_str(&l).unwrap();
                if resp["type"] == "result" || resp["type"] == "error" {
                    break;
                }
                if cmd == "start" || cmd == "ping" {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }
    }
}
