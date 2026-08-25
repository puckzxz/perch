//! Print chat events for a channel, with no UI in the way.
//!
//!     cargo run -p twitch-chat --example tail_chat -- <channel> [seconds]

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

    let (_client, mut events) = ChatClient::connect(&channel);
    println!("listening to #{channel} for {secs}s…\n");

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
