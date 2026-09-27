//! The main-thread event loop the tray icon needs.
//!
//! Windows and macOS need their native loop (via tao); on Linux the KSNI tray runs its own
//! D-Bus thread, so a plain channel is enough and no GTK is required.

use std::time::Duration;

use anyhow::Result;
use tray_icon::menu::{MenuEvent, MenuId};

use crate::process::WatchEvent;

#[derive(Debug)]
pub enum AppEvent {
    Menu(MenuId),
    Watch(WatchEvent),
    /// Ctrl+C, SIGTERM, SIGHUP.
    Terminate,
}

pub trait Handler {
    /// Called once the loop is running; tray icons must be created from here (macOS).
    fn start(&mut self);
    /// Handles an event; returns false to stop the loop.
    fn event(&mut self, event: AppEvent) -> bool;
    /// Called every tick interval; returns false to stop the loop.
    fn tick(&mut self) -> bool;
}

/// Sends events into the loop from any thread.
#[derive(Clone)]
pub struct EventSender(Inner);

#[cfg(target_os = "linux")]
type Inner = std::sync::mpsc::Sender<AppEvent>;
#[cfg(not(target_os = "linux"))]
type Inner = tao::event_loop::EventLoopProxy<AppEvent>;

impl EventSender {
    pub fn send(&self, event: AppEvent) {
        // Fails only after the loop has ended, when nobody listens anymore.
        #[cfg(target_os = "linux")]
        let _ = self.0.send(event);
        #[cfg(not(target_os = "linux"))]
        let _ = self.0.send_event(event);
    }
}

fn forward_menu_events(sender: &EventSender) {
    let sender = std::sync::Mutex::new(sender.clone());
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Ok(sender) = sender.lock() {
            sender.send(AppEvent::Menu(event.id));
        }
    }));
}

/// Runs the loop until the handler stops it and returns the handler. `make` creates the
/// handler with a sender for background threads.
#[cfg(target_os = "linux")]
pub fn run<H, F>(tick: Duration, make: F) -> Result<H>
where
    H: Handler,
    F: FnOnce(EventSender) -> Result<H>,
{
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Instant;

    let (tx, rx) = mpsc::channel();
    let sender = EventSender(tx);
    forward_menu_events(&sender);
    let mut handler = make(sender)?;
    handler.start();
    let mut next_tick = Instant::now() + tick;
    loop {
        match rx.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
            Ok(event) => {
                if !handler.event(event) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !handler.tick() {
                    break;
                }
                next_tick = Instant::now() + tick;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    Ok(handler)
}

#[cfg(not(target_os = "linux"))]
pub fn run<H, F>(tick: Duration, make: F) -> Result<H>
where
    H: Handler,
    F: FnOnce(EventSender) -> Result<H>,
{
    use std::time::Instant;
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // A tray-only app: no Dock icon, no menu bar.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let sender = EventSender(event_loop.create_proxy());
    forward_menu_events(&sender);
    let mut handler = make(sender)?;
    event_loop.run_return(|event, _, control_flow| {
        let keep_running = match event {
            Event::NewEvents(StartCause::Init) => {
                handler.start();
                *control_flow = ControlFlow::WaitUntil(Instant::now() + tick);
                true
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                *control_flow = ControlFlow::WaitUntil(Instant::now() + tick);
                handler.tick()
            }
            Event::UserEvent(event) => handler.event(event),
            _ => true,
        };
        if !keep_running {
            *control_flow = ControlFlow::Exit;
        }
    });
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    Ok(handler)
}
