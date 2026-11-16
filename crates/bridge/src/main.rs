//! Lets the Tengen website play Piskvork brains on this computer.
//!
//! A web page cannot start programs, so this one does it for the page: it
//! starts the brains named on its command line and relays the Piskvork
//! protocol between each of them and a WebSocket on 127.0.0.1, one line per
//! message. Only pages from the allowed origins may connect, and they can
//! start nothing but the brains given here.
//!
//! The page connects to `/` for the list of brains, as JSON, and to
//! `/brain/<index>` to start one; the brain stops when the socket closes.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::{Message, WebSocket};

const DEFAULT_PORT: u16 = 5577;
const SITE: &str = "https://tengen-bice.vercel.app";
/// Pages that may always connect: the website and its development servers.
const ORIGINS: &[&str] = &[
    SITE,
    "http://localhost:5173",
    "http://127.0.0.1:5173",
    "http://localhost:4173",
    "http://127.0.0.1:4173",
];
/// How long a brain may take to quit after `END` before it is killed.
const QUIT_GRACE: Duration = Duration::from_secs(2);

const USAGE: &str = "\
Usage: tengen-bridge [--port N] [--allow-origin URL]... BRAIN...

Starts nothing until the Tengen website asks: then it runs the chosen BRAIN
(a Piskvork brain such as pbrain-rapfi.exe) in the brain's own folder and
relays the protocol between it and the page.

Options:
  --port N            Port on 127.0.0.1 to listen on (default 5577)
  --allow-origin URL  Also accept pages from URL, such as http://localhost:8080
  -h, --help          Show this help

Example:
  tengen-bridge C:\\Gomoku\\pbrain-rapfi.exe C:\\Gomoku\\pbrain-embryo.exe";

/// A brain the page may start.
#[derive(Debug, Clone, PartialEq)]
struct Brain {
    name: String,
    path: PathBuf,
}

#[derive(Debug, PartialEq)]
struct Config {
    port: u16,
    origins: Vec<String>,
    brains: Vec<Brain>,
}

/// What a connection asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Route {
    List,
    Brain(usize),
}

fn main() -> ExitCode {
    let config = match parse_args(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("tengen-bridge: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let listener = match TcpListener::bind(("127.0.0.1", config.port)) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("tengen-bridge: cannot listen on 127.0.0.1:{}: {e}", config.port);
            return ExitCode::FAILURE;
        }
    };
    let port = listener.local_addr().map_or(config.port, |a| a.port());
    println!(
        "tengen-bridge {} is listening on ws://127.0.0.1:{port}",
        env!("CARGO_PKG_VERSION")
    );
    for brain in &config.brains {
        println!("  {}: {}", brain.name, brain.path.display());
    }
    println!(
        "Open {SITE}, choose \"Watch engines\" and \"On this computer\"{}.",
        if port == DEFAULT_PORT {
            String::new()
        } else {
            format!(", with port {port}")
        }
    );
    serve(listener, Arc::new(config));
    ExitCode::SUCCESS
}

/// Reads the command line; `None` asks for the help text.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Config>, String> {
    let mut port = DEFAULT_PORT;
    let mut origins: Vec<String> = ORIGINS.iter().map(|o| o.to_string()).collect();
    let mut paths = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--port" => {
                let value = args.next().ok_or("--port needs a number")?;
                port = value.parse().map_err(|_| format!("bad port {value}"))?;
            }
            "--allow-origin" => {
                let value = args.next().ok_or("--allow-origin needs a URL")?;
                origins.push(value.trim_end_matches('/').to_string());
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option {arg}")),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        return Err("name at least one brain".to_string());
    }
    let mut brains: Vec<Brain> = Vec::new();
    for path in paths {
        if !path.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        let path = std::path::absolute(&path).unwrap_or(path);
        let mut name = brain_name(&path);
        if brains.iter().any(|b| b.name == name) {
            name = format!("{name} ({})", brains.len() + 1);
        }
        brains.push(Brain { name, path });
    }
    Ok(Some(Config { port, origins, brains }))
}

/// A brain's display name: its file name without extension or `pbrain-`.
fn brain_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    stem.strip_prefix("pbrain-").unwrap_or(&stem).to_string()
}

/// The route for a request path, if it names one.
fn route(path: &str, brains: usize) -> Option<Route> {
    match path.trim_end_matches('/') {
        "" => Some(Route::List),
        p => {
            let index: usize = p.strip_prefix("/brain/")?.parse().ok()?;
            (index < brains).then_some(Route::Brain(index))
        }
    }
}

/// Accepts connections until the process is stopped, one thread each.
fn serve(listener: TcpListener, config: Arc<Config>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let config = Arc::clone(&config);
        std::thread::spawn(move || {
            if let Err(message) = handle(stream, &config) {
                eprintln!("tengen-bridge: {message}");
            }
        });
    }
}

// The handshake callback's error type is tungstenite's.
#[allow(clippy::result_large_err)]
fn handle(stream: TcpStream, config: &Config) -> Result<(), String> {
    let mut chosen = None;
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|o| o.to_str().ok())
            .unwrap_or("");
        if !config.origins.iter().any(|o| o == origin) {
            return Err(refusal(StatusCode::FORBIDDEN, "this page may not use the bridge"));
        }
        match route(request.uri().path(), config.brains.len()) {
            Some(r) => {
                chosen = Some(r);
                Ok(response)
            }
            None => Err(refusal(StatusCode::NOT_FOUND, "no such brain")),
        }
    };
    let mut ws = tungstenite::accept_hdr(stream, check).map_err(|e| format!("refused a connection: {e}"))?;
    match chosen {
        Some(Route::List) => {
            let _ = ws.send(Message::text(brain_list(&config.brains)));
            let _ = ws.close(None);
            let _ = ws.flush();
            Ok(())
        }
        Some(Route::Brain(index)) => {
            let brain = &config.brains[index];
            println!("{}: started", brain.name);
            let result = relay(ws, brain);
            println!("{}: stopped", brain.name);
            result.map_err(|e| format!("{}: {e}", brain.name))
        }
        None => Ok(()),
    }
}

fn refusal(status: StatusCode, message: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(message.to_string()));
    *response.status_mut() = status;
    response
}

/// `{"version":"…","brains":[{"name":"…"},…]}`.
fn brain_list(brains: &[Brain]) -> String {
    let items: Vec<String> = brains
        .iter()
        .map(|b| format!("{{\"name\":{}}}", json_string(&b.name)))
        .collect();
    format!(
        "{{\"version\":{},\"brains\":[{}]}}",
        json_string(env!("CARGO_PKG_VERSION")),
        items.join(",")
    )
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Runs `brain` and passes lines both ways until either side ends.
fn relay(mut ws: WebSocket<TcpStream>, brain: &Brain) -> Result<(), String> {
    let folder = brain.path.parent().unwrap_or(Path::new("."));
    let mut child = Command::new(&brain.path)
        .current_dir(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", brain.path.display()))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    // `None` once the brain's output ends.
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(Some(line.trim_end().to_string())).is_err() {
                return;
            }
        }
        let _ = tx.send(None);
    });

    // Short read timeouts let one thread watch both the socket and the brain.
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(10)))
        .map_err(|e| e.to_string())?;
    let result = loop {
        let mut brain_ended = false;
        let mut failure = None;
        loop {
            match rx.try_recv() {
                Ok(Some(line)) => {
                    if let Err(e) = ws.send(Message::text(line)) {
                        failure = Some(e.to_string());
                        break;
                    }
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    brain_ended = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if let Some(e) = failure {
            break Err(e);
        }
        if brain_ended {
            let _ = ws.close(None);
            let _ = ws.flush();
            break Ok(());
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                let mut sent = Ok(());
                for line in text.lines() {
                    sent = writeln!(stdin, "{}", line.trim_end());
                    if sent.is_err() {
                        break;
                    }
                }
                if let Err(e) = sent.and_then(|_| stdin.flush()) {
                    break Err(format!("the brain stopped reading: {e}"));
                }
            }
            Ok(Message::Close(_)) => break Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break Ok(()),
            Err(e) => break Err(e.to_string()),
        }
    };
    quit(child, stdin);
    result
}

/// Asks the brain to quit, as Piskvork managers do, and kills it if it does
/// not.
fn quit(mut child: Child, mut stdin: ChildStdin) {
    let _ = writeln!(stdin, "END");
    let _ = stdin.flush();
    drop(stdin);
    let started = Instant::now();
    while started.elapsed() < QUIT_GRACE {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn this_exe() -> String {
        std::env::current_exe().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn reads_options_and_brains() {
        let exe = this_exe();
        let config = parse_args(args(&[
            "--port",
            "6000",
            "--allow-origin",
            "http://localhost:8080/",
            &exe,
            &exe,
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(config.port, 6000);
        assert!(config.origins.iter().any(|o| o == "http://localhost:8080"));
        assert!(config.origins.iter().any(|o| o == SITE));
        assert_eq!(config.brains.len(), 2);
        assert_ne!(config.brains[0].name, config.brains[1].name);
    }

    #[test]
    fn refuses_bad_command_lines() {
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["--port", "x", &this_exe()])).is_err());
        assert!(parse_args(args(&["--colour", &this_exe()])).is_err());
        assert!(parse_args(args(&["no-such-brain.exe"])).is_err());
        assert_eq!(parse_args(args(&["--help"])), Ok(None));
    }

    #[test]
    fn names_brains_after_their_files() {
        assert_eq!(brain_name(Path::new("/x/pbrain-rapfi.exe")), "rapfi");
        assert_eq!(brain_name(Path::new("/x/embryo")), "embryo");
    }

    #[test]
    fn routes_paths() {
        assert_eq!(route("/", 2), Some(Route::List));
        assert_eq!(route("", 2), Some(Route::List));
        assert_eq!(route("/brain/1", 2), Some(Route::Brain(1)));
        assert_eq!(route("/brain/2", 2), None);
        assert_eq!(route("/brain/x", 2), None);
        assert_eq!(route("/etc/passwd", 2), None);
    }

    #[test]
    fn lists_brains_as_json() {
        let brains = [Brain {
            name: "a \"b\"\\".to_string(),
            path: PathBuf::new(),
        }];
        let json = brain_list(&brains);
        assert!(json.contains(r#""brains":[{"name":"a \"b\"\\"}]"#), "{json}");
    }

    /// Starts a bridge for `brains` on a free port.
    fn bridge(brains: Vec<Brain>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Config {
            port,
            origins: ORIGINS.iter().map(|o| o.to_string()).collect(),
            brains,
        };
        std::thread::spawn(move || serve(listener, Arc::new(config)));
        port
    }

    fn connect(port: u16, path: &str, origin: &str) -> tungstenite::Result<WebSocket<TcpStream>> {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let request = Request::builder()
            .uri(format!("ws://127.0.0.1:{port}{path}"))
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Origin", origin)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .body(())
            .unwrap();
        match tungstenite::client(request, stream) {
            Ok((ws, _)) => Ok(ws),
            Err(tungstenite::HandshakeError::Failure(e)) => Err(e),
            Err(e) => panic!("handshake: {e}"),
        }
    }

    #[test]
    fn only_allowed_pages_connect() {
        let port = bridge(vec![]);
        assert!(connect(port, "/", "https://evil.example").is_err());
        assert!(connect(port, "/brain/0", SITE).is_err());
        let mut ws = connect(port, "/", SITE).unwrap();
        let list = ws.read().unwrap().into_text().unwrap();
        assert!(list.contains("\"brains\":[]"), "{list}");
    }

    #[cfg(unix)]
    #[test]
    fn relays_a_brain() {
        use std::os::unix::fs::PermissionsExt;
        // A brain that answers START with OK and BEGIN with the center.
        let dir = std::env::temp_dir().join(format!("tengen-bridge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pbrain-echo");
        std::fs::write(
            &path,
            "#!/bin/sh\nwhile read -r line; do\n  case \"$line\" in\n    START*) echo OK ;;\n    BEGIN*) echo 7,7 ;;\n    END*) exit 0 ;;\n  esac\ndone\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let port = bridge(vec![Brain {
            name: "echo".to_string(),
            path,
        }]);
        let mut ws = connect(port, "/brain/0", SITE).unwrap();
        ws.send(Message::text("START 15")).unwrap();
        assert_eq!(ws.read().unwrap().into_text().unwrap(), "OK");
        ws.send(Message::text("BEGIN")).unwrap();
        assert_eq!(ws.read().unwrap().into_text().unwrap(), "7,7");
        // END makes the brain quit, which closes the socket.
        ws.send(Message::text("END")).unwrap();
        loop {
            match ws.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
