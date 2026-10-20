mod activity;
mod client;
mod daemon;
mod layout;
mod markup;
mod model;
mod rpc;
mod search;
mod store;
mod ui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("daemon") => daemon::run().await,
        Some("--help" | "-h") => {
            println!("Codex Deck {} — Codex multi-session TUI\n\nUsage: codex-deck [--demo|--render-demo|status|shutdown]\n\nStart without arguments for the interactive workspace.\n/side [question] or /btw [question] opens a right-hand side conversation.\nCtrl+G, then q detaches; tasks continue in the background.\n\nEnvironment:\n  CODEX_DECK_CODEX  Codex CLI executable (default: codex)\n  CODEX_DECK_HOME   Private state directory (default: XDG state/sessiondesk)\n\nLegacy SESSIONDESK_CODEX / SESSIONDESK_HOME remain supported.\n\nRequires Codex CLI with app-server support and an interactive UTF-8 terminal.",env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("codex-deck {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--demo") => ui::run_demo(),
        Some("--render-demo") => ui::render_demo(),
        Some(op @ ("status" | "shutdown")) => {
            let mut c = client::Client::attach().await?;
            let id = c.send(
                if op == "status" { "snapshot" } else { op },
                serde_json::json!({}),
            );
            loop {
                let v = c.rx.recv()?;
                if v["id"] == id {
                    if v.get("error").is_some() {
                        anyhow::bail!("{}", v["error"])
                    }
                    if op == "status" {
                        let s: &serde_json::Value = &v["result"];
                        println!(
                            "connected={} conversations={} pending={} state={}",
                            s["connected"],
                            s["data"]["conversations"]
                                .as_object()
                                .map_or(0, |m| m.len()),
                            s["pending"].as_array().map_or(0, |a| a.len()),
                            model::state_dir().display()
                        );
                        if let Some(e) = s["error"].as_str().filter(|s| !s.is_empty()) {
                            println!("{e}");
                        }
                    } else {
                        println!("Background service stopped");
                    }
                    break;
                }
            }
            Ok(())
        }
        None => {
            use std::io::IsTerminal;
            anyhow::ensure!(
                std::io::stdin().is_terminal(),
                "Run Codex Deck in an interactive terminal; use --help for commands."
            );
            ui::run(client::Client::connect().await?)
        }
        Some(other) => anyhow::bail!("Unknown option {other}; use --help"),
    }
}
