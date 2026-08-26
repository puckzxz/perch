//! Print chat events for a channel, with no UI in the way.
//!
//!     cargo run -p twitch-chat --example tail_chat -- <channel> [seconds] [history]
//!
//! `history` is the scrollback to request before joining, so the backfill and
//! the live feed can be told apart by where `[connected]` falls.

use std::time::{Duration, Instant};

use futures::StreamExt as _;
use twitch_chat::{ChatClient, ChatEvent};

fn main() {
    let Some(channel) = std::env::args().nth(1) else {
        eprintln!("usage: tail_chat <channel> [seconds]");
        std::process::exit(2);
    };
    let secs: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let history: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);

    let (_client, mut events) = ChatClient::connect(&channel, history);
    println!("listening to #{channel} for {secs}s, after {history} of backlog…\n");

    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut messages = 0usize;
    let mut others = 0usize;

    futures::executor::block_on(async {
        loop {
            if Instant::now() >= deadline {
                break;
            }
            match futures::future::select(
                events.next(),
                Box::pin(futures_timer::Delay::new(Duration::from_millis(500))),
            )
            .await
            {
                futures::future::Either::Left((Some(event), _)) => match event {
                    ChatEvent::Connected { channel } => {
                        others += 1;
                        println!("[connected] #{channel}");
                    }
                    ChatEvent::Message(m) => {
                        messages += 1;
                        println!("<{}> {}", m.display_name, m.text);
                    }
                    ChatEvent::Notice(n) => {
                        others += 1;
                        println!("[{:?}] {}", n.kind, n.system);
                        if let Some(body) = n.body {
                            println!("        <{}> {}", body.display_name, body.text);
                        }
                    }
                    ChatEvent::RoomState { room_id } => {
                        others += 1;
                        println!("[room] id={room_id}");
                    }
                    ChatEvent::Cleared { login } => {
                        others += 1;
                        println!("[cleared] {login:?}");
                    }
                    ChatEvent::Disconnected { reason } => {
                        others += 1;
                        println!("[disconnected] {reason}");
                    }
                },
                futures::future::Either::Left((None, _)) => {
                    println!("[event stream ended]");
                    break;
                }
                futures::future::Either::Right(_) => {}
            }
        }
    });

    println!("\n{messages} message(s), {others} other event(s) in {secs}s");
}
