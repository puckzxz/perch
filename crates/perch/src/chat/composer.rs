//! The box at the foot of a live chat that sends a message.
//!
//! Reading stays anonymous IRC, as it always was; sending goes through
//! Helix's Send Chat Message with the signed-in user's token
//! (`twitch_api::chat`), asked of the Twitch worker
//! (`twitch::Request::SendChat`) so the window never waits on it. A message
//! sent here is not drawn when it is sent: it appears when IRC echoes it,
//! like anybody else's, so what the pane shows is what the room got. Only
//! what never reaches the room is said here, as a notice row in the chat
//! it was typed into: Twitch's own sentence when it answered and did not
//! send (`is_sent` false, [`outcome_notice`]), or a short one of ours when
//! it refused or could not be asked ([`failure_notice`]).
//!
//! A live chat only: a recording's replay was said last week, and nothing
//! typed under it would land beside it. The box is gpui-component's
//! `Input`, as the app's other text boxes are, so while it has the cursor
//! the shortcuts stand aside for it (`keys::TYPING`), all but `Ctrl+K`,
//! `Ctrl+,` and `Ctrl+R`, which type nothing: the same rule that keeps
//! `Space` typing a space in the title bar's search box; nothing here
//! claims a key of its own beyond the box's `Enter`. `Enter` sends what is
//! in it, trimmed, and empties it; the box takes no line breaks (a pasted
//! one becomes a space) and stops at Twitch's 500 characters ([`fit`],
//! [`message`]).
//!
//! When the user cannot send, the box gives way to one line saying why,
//! with the thing to do beside it where there is one ([`state`], pure and
//! tested): signed out, `Sign in to chat`; a sign-in from before sending
//! was asked for, `Sign in again to chat`; a followers-only room the user
//! is known not to follow. The modes the user may or may not be exempt
//! from (sub-only, which nothing here can check; followers-only while
//! following, for an unknown number of minutes; emote-only) leave the box
//! open and say the mode in its placeholder, and Twitch's drop reason says
//! the rest if it refuses.
//!
//! It sits under the line of modes, in the chat's own column, so it takes
//! its height from the list's box and never from a row: the list stays in
//! its absolute layer and keeps every measurement it has (HANDOFF, the
//! "view flowed into a flex item" trap). Its height is the box's in both
//! states (`theme::COMPOSER_HEIGHT`), so the list does not jump as the reason
//! comes and goes.

use gpui::{div, prelude::*, px, AnyElement, Context, Entity, SharedString, Subscription, Window};
use gpui_component::input::{Input, InputEvent, InputState};
use twitch_api::chat::SendOutcome;
use twitch_chat::RoomModes;

use super::{ChatView, ChatViewEvent, RowKind};
use crate::browse::SignIn;
use crate::controls;
use crate::theme;
use crate::twitch::SendFailure;

/// The most Twitch takes in one message, in characters.
pub const MAX_CHARS: usize = 500;

/// What the placeholder says with no room mode in the way.
const PLACEHOLDER: &str = "Send a message";

/// Where the user's sign-in stands, as far as sending is concerned: the
/// root's `SignIn`, with whether the token may send folded in
/// ([`Account::of`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    Connecting,
    NeedsClientId,
    SignedOut,
    AwaitingCode {
        user_code: SharedString,
        verification_uri: SharedString,
    },
    /// Sign-in failed, and the worker has stopped.
    Failed,
    /// Signed in with a token from before sending was asked for.
    NeedsScope,
    /// Signed in, and the token may send, or nobody knows yet: validation
    /// had not answered, and a send finds out.
    Ready,
}

impl Account {
    /// `scope` is whether the token may send chat, `None` until Twitch's
    /// token validation has said (`TwitchEvent::ChatScope`).
    pub fn of(sign_in: &SignIn, scope: Option<bool>) -> Self {
        match sign_in {
            SignIn::Connecting => Account::Connecting,
            SignIn::NeedsClientId => Account::NeedsClientId,
            SignIn::SignedOut => Account::SignedOut,
            SignIn::AwaitingCode {
                user_code,
                verification_uri,
            } => Account::AwaitingCode {
                user_code: user_code.clone(),
                verification_uri: verification_uri.clone(),
            },
            SignIn::Error(_) => Account::Failed,
            SignIn::SignedIn(_) if scope == Some(false) => Account::NeedsScope,
            SignIn::SignedIn(_) => Account::Ready,
        }
    }
}

/// What a live chat's composer needs from the root to decide whether it is
/// open: the sign-in, and two facts about the user and this channel. The
/// root's, mirrored into each live chat as they change
/// ([`ChatView::set_access`], `RootView::sync_chat_access`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub account: Account,
    /// The chat is the signed-in user's own channel. No room mode holds its
    /// broadcaster back.
    pub own_channel: bool,
    /// Whether the user follows this channel: `None` until a follows list
    /// has come back, or signed out.
    pub follows: Option<bool>,
}

/// [`Access::follows`] from the follows list: `None` until one has come
/// back (`loaded`), and then whether the channel was `found` in it. Except
/// that a list cut short at its page cap (not `complete`; a thousand
/// channels, `twitch_api::Followed`) cannot say somebody is not followed,
/// only that they are, so a channel missing from it is still `None`: the
/// box stays open with the followers-only hint, and Twitch's drop reason
/// has the last word, rather than shutting out a follow on page eleven.
pub fn follows(loaded: bool, complete: bool, found: bool) -> Option<bool> {
    match (loaded, found) {
        (false, _) => None,
        (true, true) => Some(true),
        (true, false) => complete.then_some(false),
    }
}

impl Default for Access {
    fn default() -> Self {
        Self {
            account: Account::Connecting,
            own_channel: false,
            follows: None,
        }
    }
}

/// What the composer is: a box to type into, saying `placeholder` while
/// empty, or one line saying why not, with the thing to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Open {
        placeholder: &'static str,
    },
    Closed {
        reason: SharedString,
        step: Option<Step>,
    },
}

/// The button beside a closed composer's reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Start the device-code sign-in (`RootView::start_sign_in`).
    SignIn,
    /// Forget the old sign-in and start a new one, for the scope it lacks
    /// (`RootView::sign_in_again`).
    SignInAgain,
    /// The settings sheet, where the client id goes.
    OpenSettings,
    /// The page the sign-in code is typed into, with the code filled in.
    OpenActivate(SharedString),
}

impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Step::SignIn => "Sign in",
            Step::SignInAgain => "Sign in again",
            Step::OpenSettings => "Open settings",
            Step::OpenActivate(_) => "Open Twitch",
        }
    }
}

/// Whether the composer is open and what it says, from what the root knows
/// (`access`), whether the room's id has arrived (`room_known`: a send
/// needs it), and the room's modes as last heard.
///
/// The modes close the box only where the answer is known: followers-only
/// for a user the follows list says does not follow. Moderators and VIPs
/// are exempt there too, and nothing here can tell, which is the price of
/// not inviting a message Twitch will only drop. Sub-only cannot be
/// checked at all — no scope this app asks for says who subscribes — so it,
/// followers-only while following (the minutes are unknown) and emote-only
/// open the box and name the mode in its placeholder. The broadcaster is
/// held back by none of them.
pub fn state(access: &Access, room_known: bool, modes: Option<&RoomModes>) -> State {
    let closed = |reason: SharedString, step| State::Closed { reason, step };
    match &access.account {
        Account::Connecting => return closed("Connecting to Twitch…".into(), None),
        Account::NeedsClientId => {
            return closed("Sign in to chat".into(), Some(Step::OpenSettings))
        }
        Account::SignedOut | Account::Failed => {
            return closed("Sign in to chat".into(), Some(Step::SignIn))
        }
        Account::AwaitingCode {
            user_code,
            verification_uri,
        } => {
            return closed(
                format!("Sign in with code {user_code}").into(),
                Some(Step::OpenActivate(verification_uri.clone())),
            )
        }
        Account::NeedsScope => {
            return closed("Sign in again to chat".into(), Some(Step::SignInAgain))
        }
        Account::Ready => {}
    }
    if !room_known {
        return closed("Joining the chat…".into(), None);
    }
    let modes = match modes {
        Some(modes) if !access.own_channel => *modes,
        _ => RoomModes::default(),
    };
    if modes.followers_only.is_some() && access.follows == Some(false) {
        return closed("Only followers can chat here".into(), None);
    }
    let placeholder = if modes.subs_only {
        "Send a message (sub-only chat)"
    } else if modes.followers_only.is_some() {
        "Send a message (followers-only chat)"
    } else if modes.emote_only {
        "Send an emote (emote-only chat)"
    } else {
        PLACEHOLDER
    };
    State::Open { placeholder }
}

/// Why a message cannot go as typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// Nothing but spaces. Not said: `Enter` on an empty box does nothing.
    Empty,
    /// Over [`MAX_CHARS`]. The box stops there ([`fit`]), so this is the
    /// last guard rather than something a user meets.
    TooLong,
}

/// What `typed` sends: trimmed, with any line break a space, as long as it
/// is something and no longer than Twitch takes.
pub fn message(typed: &str) -> Result<String, Invalid> {
    let flat = typed.replace(['\r', '\n'], " ");
    let trimmed = flat.trim();
    if trimmed.is_empty() {
        return Err(Invalid::Empty);
    }
    if trimmed.chars().count() > MAX_CHARS {
        return Err(Invalid::TooLong);
    }
    Ok(trimmed.to_string())
}

/// What the box should hold instead of `typed`, if anything: line breaks
/// made spaces, and cut at [`MAX_CHARS`]. `None` when it already fits.
///
/// Applied on every change rather than as the box's validator, which turns
/// a change away whole: a paste of 600 characters, or of two lines from a
/// Windows clipboard (`\r\n`, of which the box itself strips only the
/// `\n`), would have done nothing at all.
pub fn fit(typed: &str) -> Option<String> {
    let fits = typed.chars().count() <= MAX_CHARS && !typed.contains(['\r', '\n']);
    if fits {
        return None;
    }
    Some(
        typed
            .replace("\r\n", " ")
            .replace(['\r', '\n'], " ")
            .chars()
            .take(MAX_CHARS)
            .collect(),
    )
}

/// The notice a send's answer leaves in its chat, if any: none when it was
/// sent, since IRC's echo is the proof; Twitch's own reason when it was
/// dropped.
pub fn outcome_notice(outcome: &SendOutcome) -> Option<String> {
    let notice = match outcome {
        SendOutcome::Sent => return None,
        SendOutcome::Dropped { message, .. } if !message.trim().is_empty() => {
            message.trim().to_string()
        }
        SendOutcome::Dropped { .. } => "Twitch did not send the message".to_string(),
        SendOutcome::MissingScope => "Message not sent: sign in again to chat".to_string(),
        SendOutcome::Refused { status: 403, .. } => {
            "Message not sent: you are not allowed to chat here".to_string()
        }
        SendOutcome::Refused { status: 422, .. } => "Message not sent: it is too long".to_string(),
        SendOutcome::Refused { status: 429, .. } => {
            "Message not sent: you are sending messages too quickly".to_string()
        }
        SendOutcome::Refused {
            status,
            message: Some(message),
        } => format!("Message not sent: {message} (HTTP {status})"),
        SendOutcome::Refused {
            status,
            message: None,
        } => format!("Message not sent: Twitch said HTTP {status}"),
    };
    Some(notice)
}

/// The notice a send that got no answer leaves in its chat.
pub fn failure_notice(failure: &SendFailure) -> String {
    match failure {
        SendFailure::Network => "Message not sent: could not reach Twitch".to_string(),
        SendFailure::SignIn => "Message not sent: Twitch did not accept the sign-in".to_string(),
        SendFailure::Other(reason) => format!("Message not sent: {reason}"),
    }
}

/// A live chat's composer: the box, and what the root last said about the
/// user ([`Access`]).
pub(super) struct Composer {
    input: Entity<InputState>,
    access: Access,
    /// The placeholder the box was last given, so it is set again only
    /// when a mode changes it.
    placeholder: &'static str,
    _events: Subscription,
}

impl Composer {
    pub(super) fn new(window: &mut Window, cx: &mut Context<ChatView>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER));
        let events = cx.subscribe_in(
            &input,
            window,
            |chat: &mut ChatView, input, event, window, cx| match event {
                InputEvent::Change => {
                    let fitted = fit(&input.read(cx).value());
                    if let Some(fitted) = fitted {
                        input.update(cx, |input, cx| input.set_value(fitted, window, cx));
                    }
                }
                InputEvent::PressEnter { .. } => chat.submit(window, cx),
                _ => {}
            },
        );
        Self {
            input,
            access: Access::default(),
            placeholder: PLACEHOLDER,
            _events: events,
        }
    }
}

impl ChatView {
    /// What the root knows about the user, for this chat's composer:
    /// mirrored on every change of the sign-in, the scope and the follows
    /// (`RootView::sync_chat_access`). Nothing for a replay, which has no
    /// composer, or for one told what it already knew.
    pub fn set_access(&mut self, access: Access, cx: &mut Context<Self>) {
        let Some(composer) = self.composer.as_mut() else {
            return;
        };
        if composer.access == access {
            return;
        }
        composer.access = access;
        cx.notify();
    }

    /// The channel a live chat is in, by login; `None` for a replay.
    pub fn live_channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }

    /// What became of a message this chat sent, said as a notice row when
    /// it did not reach the room (`RootView::on_chat_sent`).
    pub fn sent(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        if let Some(notice) = notice {
            self.append(RowKind::Notice(notice.into()), None);
            cx.notify();
        }
    }

    /// The composer's state now, or `None` for a replay.
    fn composer_state(&self) -> Option<State> {
        let composer = self.composer.as_ref()?;
        Some(state(
            &composer.access,
            self.room_id.is_some(),
            self.modes.as_ref(),
        ))
    }

    /// `Enter` in the box: send what it holds, if the box is open and it is
    /// something, and empty it. The root does the sending
    /// ([`ChatViewEvent::Send`]).
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.composer_state(), Some(State::Open { .. })) {
            return;
        }
        let (Some(composer), Some(room_id)) = (self.composer.as_ref(), self.room_id.clone()) else {
            return;
        };
        let Ok(text) = message(&composer.input.read(cx).value()) else {
            return;
        };
        composer
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.emit(ChatViewEvent::Send {
            room_id,
            message: text,
        });
    }

    /// The composer as drawn at the foot of the chat, under the line of
    /// modes: the box, or the line that says why not. `None` for a replay.
    pub(super) fn composer_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.composer_state()?;
        let composer = self.composer.as_mut()?;
        let bar = div()
            .flex_none()
            .w_full()
            .px(px(theme::GAP_TIGHT))
            .py(px(theme::GAP_TIGHT))
            .border_t_1()
            .border_color(theme::divider());
        let bar = match state {
            State::Open { placeholder } => {
                if composer.placeholder != placeholder {
                    composer.placeholder = placeholder;
                    composer.input.update(cx, |input, cx| {
                        input.set_placeholder(placeholder, window, cx)
                    });
                }
                bar.child(Input::new(&composer.input))
            }
            State::Closed { reason, step } => bar.child(
                div()
                    .h(px(theme::COMPOSER_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    .px(px(theme::GAP_TIGHT))
                    // One line, cut short in a narrow chat: `text_ellipsis`
                    // and `line_clamp`, the pair that truncates (HANDOFF,
                    // "Things not to redo").
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text_dim())
                            .text_ellipsis()
                            .line_clamp(1)
                            .child(reason),
                    )
                    .children(step.map(|step| {
                        controls::pill(
                            "chat-composer-step",
                            step.label(),
                            controls::Variant::Primary,
                        )
                        .on_click(cx.listener(
                            move |_, _event, _window, cx| match &step {
                                Step::OpenActivate(uri) => cx.open_url(uri),
                                step => cx.emit(ChatViewEvent::Step(step.clone())),
                            },
                        ))
                    })),
            ),
        };
        Some(bar.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Access {
        Access {
            account: Account::Ready,
            own_channel: false,
            follows: Some(true),
        }
    }

    fn reason(state: State) -> (String, Option<Step>) {
        match state {
            State::Closed { reason, step } => (reason.to_string(), step),
            State::Open { placeholder } => panic!("open, saying {placeholder}"),
        }
    }

    /// The sign-in, as the root holds it, with the scope folded in.
    /// Not followed is said only on the whole list; a list cut short at its
    /// page cap says who is followed and nothing about who is not.
    #[test]
    fn only_a_whole_follows_list_says_not_followed() {
        assert_eq!(follows(false, true, true), None);
        assert_eq!(follows(false, false, false), None);
        assert_eq!(follows(true, true, true), Some(true));
        assert_eq!(follows(true, false, true), Some(true));
        assert_eq!(follows(true, true, false), Some(false));
        assert_eq!(follows(true, false, false), None);
    }

    #[test]
    fn the_account_follows_the_sign_in_and_the_scope() {
        let signed_in = SignIn::SignedIn("me".into());
        assert_eq!(Account::of(&signed_in, None), Account::Ready);
        assert_eq!(Account::of(&signed_in, Some(true)), Account::Ready);
        assert_eq!(Account::of(&signed_in, Some(false)), Account::NeedsScope);
        assert_eq!(Account::of(&SignIn::SignedOut, None), Account::SignedOut);
        assert_eq!(
            Account::of(&SignIn::NeedsClientId, Some(false)),
            Account::NeedsClientId
        );
        assert_eq!(
            Account::of(&SignIn::Error("no".into()), None),
            Account::Failed
        );
        assert_eq!(Account::of(&SignIn::Connecting, None), Account::Connecting);
    }

    /// Every way of not being able to send says so in a line, with the
    /// thing to do where there is one.
    #[test]
    fn a_closed_composer_says_why_and_what_to_do() {
        let with = |account| Access { account, ..ready() };
        let modes = RoomModes::default();
        let closed = |account| reason(state(&with(account), true, Some(&modes)));

        assert_eq!(
            closed(Account::SignedOut),
            ("Sign in to chat".into(), Some(Step::SignIn))
        );
        assert_eq!(
            closed(Account::NeedsClientId),
            ("Sign in to chat".into(), Some(Step::OpenSettings))
        );
        assert_eq!(
            closed(Account::Failed),
            ("Sign in to chat".into(), Some(Step::SignIn))
        );
        assert_eq!(
            closed(Account::NeedsScope),
            ("Sign in again to chat".into(), Some(Step::SignInAgain))
        );
        assert_eq!(closed(Account::Connecting).1, None);
        assert_eq!(
            closed(Account::AwaitingCode {
                user_code: "ABCD1234".into(),
                verification_uri: "https://www.twitch.tv/activate?device-code=ABCD1234".into(),
            }),
            (
                "Sign in with code ABCD1234".into(),
                Some(Step::OpenActivate(
                    "https://www.twitch.tv/activate?device-code=ABCD1234".into()
                ))
            )
        );

        // Signed in, but the room has not said its id yet, and a send needs it.
        assert_eq!(reason(state(&ready(), false, None)).1, None);

        for step in [
            Step::SignIn,
            Step::SignInAgain,
            Step::OpenSettings,
            Step::OpenActivate("x".into()),
        ] {
            assert!(step.label().starts_with(char::is_uppercase));
        }
    }

    #[test]
    fn an_open_composer_names_the_mode_in_its_placeholder() {
        let open = |access: &Access, modes: RoomModes| match state(access, true, Some(&modes)) {
            State::Open { placeholder } => placeholder,
            closed => panic!("closed: {closed:?}"),
        };
        assert_eq!(open(&ready(), RoomModes::default()), "Send a message");
        assert_eq!(
            open(
                &ready(),
                RoomModes {
                    slow: 30,
                    unique: true,
                    ..Default::default()
                }
            ),
            "Send a message",
            "slow and unique chat close nothing and say nothing here"
        );
        let subs = RoomModes {
            subs_only: true,
            ..Default::default()
        };
        assert_eq!(open(&ready(), subs), "Send a message (sub-only chat)");
        let followers = RoomModes {
            followers_only: Some(10),
            ..Default::default()
        };
        assert_eq!(
            open(&ready(), followers),
            "Send a message (followers-only chat)"
        );
        let emotes = RoomModes {
            emote_only: true,
            ..Default::default()
        };
        assert_eq!(open(&ready(), emotes), "Send an emote (emote-only chat)");
        // Before the room has said anything about itself.
        assert!(matches!(state(&ready(), true, None), State::Open { .. }));

        // The broadcaster is held back by none of them.
        let own = Access {
            own_channel: true,
            follows: Some(false),
            ..ready()
        };
        assert_eq!(open(&own, followers), "Send a message");
        assert_eq!(open(&own, subs), "Send a message");
    }

    /// Followers-only closes the box only for someone known not to follow.
    #[test]
    fn followers_only_closes_only_for_someone_known_not_to_follow() {
        let followers = RoomModes {
            followers_only: Some(0),
            ..Default::default()
        };
        let stranger = Access {
            follows: Some(false),
            ..ready()
        };
        assert_eq!(
            reason(state(&stranger, true, Some(&followers))),
            ("Only followers can chat here".into(), None)
        );
        let unknown = Access {
            follows: None,
            ..ready()
        };
        assert!(matches!(
            state(&unknown, true, Some(&followers)),
            State::Open { .. }
        ));
        assert!(matches!(
            state(&stranger, true, Some(&RoomModes::default())),
            State::Open { .. }
        ));
    }

    #[test]
    fn a_message_is_trimmed_flat_and_at_most_500_characters() {
        assert_eq!(message("  hello there  ").as_deref(), Ok("hello there"));
        assert_eq!(
            message("one\r\ntwo\nthree").as_deref(),
            Ok("one  two three")
        );
        assert_eq!(message(""), Err(Invalid::Empty));
        assert_eq!(message("   \r\n "), Err(Invalid::Empty));
        let most = "é".repeat(MAX_CHARS);
        assert_eq!(
            message(&most).as_deref(),
            Ok(most.as_str()),
            "characters, not bytes"
        );
        assert_eq!(message(&format!("{most}x")), Err(Invalid::TooLong));
        assert_eq!(
            message(&format!("  {most}  ")).as_deref(),
            Ok(most.as_str()),
            "the spaces trimmed do not count"
        );
    }

    #[test]
    fn the_box_takes_no_line_breaks_and_stops_at_the_limit() {
        assert_eq!(fit("hello"), None);
        assert_eq!(fit(&"a".repeat(MAX_CHARS)), None);
        assert_eq!(fit("one\r\ntwo").as_deref(), Some("one two"));
        assert_eq!(fit("one\rtwo\nthree").as_deref(), Some("one two three"));
        let long = "é".repeat(MAX_CHARS + 20);
        let fitted = fit(&long).unwrap();
        assert_eq!(fitted.chars().count(), MAX_CHARS);
        assert_eq!(fit(&fitted), None, "what it fits to fits");
    }

    #[test]
    fn a_send_that_did_not_land_says_why_in_the_chat() {
        assert_eq!(outcome_notice(&SendOutcome::Sent), None);
        assert_eq!(
            outcome_notice(&SendOutcome::Dropped {
                code: "msg_subsonly".into(),
                message: "This room is in subscribers-only mode.".into(),
            })
            .as_deref(),
            Some("This room is in subscribers-only mode."),
            "Twitch's own words, as they are"
        );
        assert_eq!(
            outcome_notice(&SendOutcome::Dropped {
                code: String::new(),
                message: " ".into(),
            })
            .as_deref(),
            Some("Twitch did not send the message")
        );
        assert_eq!(
            outcome_notice(&SendOutcome::MissingScope).as_deref(),
            Some("Message not sent: sign in again to chat")
        );
        let refused = |status, message: Option<&str>| {
            outcome_notice(&SendOutcome::Refused {
                status,
                message: message.map(str::to_string),
            })
            .unwrap()
        };
        assert_eq!(
            refused(429, Some("Too Many Requests")),
            "Message not sent: you are sending messages too quickly"
        );
        assert_eq!(
            refused(403, None),
            "Message not sent: you are not allowed to chat here"
        );
        assert_eq!(refused(422, None), "Message not sent: it is too long");
        assert_eq!(
            refused(400, Some("Bad thing")),
            "Message not sent: Bad thing (HTTP 400)"
        );
        assert_eq!(refused(418, None), "Message not sent: Twitch said HTTP 418");

        assert_eq!(
            failure_notice(&SendFailure::Network),
            "Message not sent: could not reach Twitch"
        );
        assert_eq!(
            failure_notice(&SendFailure::SignIn),
            "Message not sent: Twitch did not accept the sign-in"
        );
        assert_eq!(
            failure_notice(&SendFailure::Other("odd".into())),
            "Message not sent: odd"
        );
    }
}
