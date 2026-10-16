//! Test double for the `tailscale` CLI. Appends each argv (space-joined) to $FAKE_TAILSCALE_LOG; answers
//! `status --json` with the contents of $FAKE_TAILSCALE_STATUS; fails like a stopped daemon when
//! $FAKE_TAILSCALE_DOWN is set.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(log) = std::env::var("FAKE_TAILSCALE_LOG") {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(log).expect("log");
        writeln!(f, "{}", args.join(" ")).expect("write log");
    }
    if std::env::var_os("FAKE_TAILSCALE_DOWN").is_some() {
        eprintln!("failed to connect to local Tailscale daemon for /localapi/v0/status; not running?");
        std::process::exit(1);
    }
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status", "--json"] => {
            let file = std::env::var("FAKE_TAILSCALE_STATUS").expect("FAKE_TAILSCALE_STATUS");
            print!("{}", std::fs::read_to_string(file).expect("status file"));
        }
        ["serve", ..] => {}
        other => {
            eprintln!("fake-tailscale: unexpected arguments {other:?}");
            std::process::exit(2);
        }
    }
}
