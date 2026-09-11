//! `poll(2)` over the touchscreen and the bezel page-button device at once,
//! surfacing one [`InputEvent`]. The ready fd drains without blocking on the
//! other; `Touch::next_event` answers `None` short of a boundary and re-polls.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::buttons::{Buttons, PageButton};
use super::touch::{Touch, TouchEvent};
use crate::orientation::Orientation;

/// How long [`Input::next`] blocks before surfacing a `Tick`, bounding how
/// quickly a device rotation reaches the main loop. Fires on an idle poll:
/// real input returns first.
const TICK_MS: libc::c_int = 500;

/// How long [`Input::follow_orientation`] leaves between `detect` reads. Each
/// is a `lipc-get-prop` spawn, so it is not worth doing on every `Tick`.
const ORIENT_POLL: Duration = Duration::from_millis(1000);

/// A unified input event from either device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Touch(TouchEvent),
    Page(PageButton),
    /// `poll` timed out with no input. The main loop re-checks orientation
    /// here: the X server rotates the display, raw evdev coords stay fixed.
    Tick,
}

pub struct Input {
    touch: Touch,
    /// `None` when no page-button device was found/openable — the picker runs
    /// touch-only and `poll` watches just the touchscreen.
    buttons: Option<Buttons>,
    /// The orientation both devices are set to.
    orientation: Orientation,
    /// When `orientation` was read; `None` asks for a read at once.
    checked: Option<Instant>,
    /// [`Input::set_covered`]'s state.
    covered: bool,
}

impl Input {
    pub fn new(touch: Touch, buttons: Option<Buttons>) -> Self {
        Self {
            touch,
            buttons,
            orientation: Orientation::Up,
            checked: None,
            covered: false,
        }
    }

    /// What the two readers resolved to, for the log's header block.
    pub fn describe(&self) -> String {
        format!(
            "{} {}",
            self.touch.describe(),
            super::buttons::describe(self.buttons.as_ref()),
        )
    }

    /// Re-orient both devices after a detected rotation (the display is rotated
    /// by the X server; raw evdev coords/buttons are panel-fixed and need this).
    pub fn set_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
        self.touch.set_orientation(orientation);
        if let Some(buttons) = self.buttons.as_mut() {
            buttons.set_orientation(orientation);
        }
    }

    /// [`Touch::set_covered`] and [`Buttons::set_covered`] over both devices,
    /// dropping the `EVIOCGRAB` that would otherwise leave the passcode and
    /// the ads screen drawn with nothing able to reach them.
    pub fn set_covered(&mut self, covered: bool) {
        self.covered = covered;
        if !covered {
            self.checked = None;
        }
        self.touch.set_covered(covered);
        if let Some(buttons) = self.buttons.as_mut() {
            buttons.set_covered(covered);
        }
    }

    /// [`Touch::retake`] and [`Buttons::retake`] over both devices, for a grab
    /// another reader held when this app wanted it.
    pub fn retake(&mut self) {
        self.touch.retake();
        if let Some(buttons) = self.buttons.as_mut() {
            buttons.retake();
        }
    }

    /// [`Touch::set_size`] after a relayout. Nothing on the button device is
    /// measured against the screen, so this reaches the touchscreen alone.
    pub fn set_size(&mut self, fb_xres: u32, fb_yres: u32) {
        self.touch.set_size(fb_xres, fb_yres);
    }

    /// Re-reads [`Orientation::detect`] past [`ORIENT_POLL`] and applies a
    /// change to both devices, answering whether one landed. The only runtime
    /// read; a `true` is the caller's cue to rebuild its layout and repaint.
    pub fn follow_orientation(&mut self) -> bool {
        if self.covered {
            return false;
        }
        if let Some(at) = self.checked
            && at.elapsed() < ORIENT_POLL
        {
            return false;
        }
        self.checked = Some(Instant::now());
        let seen = Orientation::detect();
        if seen == self.orientation {
            return false;
        }
        self.set_orientation(seen);
        true
    }

    /// [`Input::follow_orientation`] with the [`ORIENT_POLL`] throttle skipped,
    /// for the reads that must not run behind: a press whose coordinates are
    /// about to be acted on.
    pub fn follow_orientation_now(&mut self) -> bool {
        self.checked = None;
        self.follow_orientation()
    }

    /// Latest primary touch position in user-visible coords, read at the arm
    /// deadline against `crate::ARM_SLOP_PX`.
    pub fn touch_pos(&self) -> (u32, u32) {
        self.touch.current_pos()
    }

    /// A zero-timeout `poll`: the first ready event, or `None`. Surfaces no
    /// `Tick`, and a partial touch stroke reads as `None`.
    pub fn poll_now(&mut self) -> Result<Option<InputEvent>> {
        let touch_fd: RawFd = self.touch.raw_fd();
        let button_fd: RawFd = self.buttons.as_ref().map(|b| b.raw_fd()).unwrap_or(-1);
        let mut fds = [
            libc::pollfd {
                fd: touch_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: button_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let nfds: libc::nfds_t = if self.buttons.is_some() { 2 } else { 1 };
        // A negative rc (EINTR, error) reads as no input this tick.
        if unsafe { libc::poll(fds.as_mut_ptr(), nfds, 0) } <= 0 {
            return Ok(None);
        }
        // Touch first, past [`Input::next`]'s order: a stale `None` button read
        // returns early and shadows a pending touch event.
        if fds[0].revents & libc::POLLIN != 0
            && let Some(ev) = self.touch.next_event()?
        {
            return Ok(Some(InputEvent::Touch(ev)));
        }
        if let Some(buttons) = self.buttons.as_mut()
            && fds[1].revents & libc::POLLIN != 0
            && let Some(page) = buttons.read_one()?
        {
            return Ok(Some(InputEvent::Page(page)));
        }
        Ok(None)
    }

    /// Block until the next event from either device (see
    /// [`Self::next_deadline`]); the everyday call, with only the idle
    /// [`TICK_MS`] wake and no arm deadline.
    pub fn next_event(&mut self) -> Result<InputEvent> {
        self.next_deadline(None)
    }

    /// [`Self::next`] with an [`InputEvent::Tick`] at `deadline`, past a busy
    /// touch fd: the timeout is the remaining time to the absolute `deadline`,
    /// recomputed each iteration. `None` gives a plain [`TICK_MS`] idle tick.
    pub fn next_deadline(&mut self, deadline: Option<Instant>) -> Result<InputEvent> {
        let touch_fd: RawFd = self.touch.raw_fd();
        loop {
            // At or past `deadline`, through move-jitter that kept `poll` busy.
            if let Some(d) = deadline
                && Instant::now() >= d
            {
                return Ok(InputEvent::Tick);
            }
            let button_fd: RawFd = self.buttons.as_ref().map(|b| b.raw_fd()).unwrap_or(-1);
            let mut fds = [
                libc::pollfd {
                    fd: touch_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: button_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let nfds: libc::nfds_t = if self.buttons.is_some() { 2 } else { 1 };

            // Remaining time to `deadline`, floored at 1ms against a sub-ms
            // spin, else [`TICK_MS`]. `poll` wakes early on fd readiness.
            let timeout = match deadline {
                Some(d) => (d
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(i32::MAX as u128) as libc::c_int)
                    .max(1),
                None => TICK_MS,
            };
            let rc = unsafe { libc::poll(fds.as_mut_ptr(), nfds, timeout) };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue; // EINTR — re-arm the poll.
                }
                return Err(err).context("poll(touch, buttons)");
            }
            if rc == 0 {
                return Ok(InputEvent::Tick); // deadline reached, or idle timeout.
            }
            // `deadline` passed while `poll` blocked, with an event in the same
            // wake: the arm wins and the event surfaces next call. A finger
            // lifting on the threshold returns `Tick`, never `Up`.
            if let Some(d) = deadline
                && Instant::now() >= d
            {
                return Ok(InputEvent::Tick);
            }

            // Buttons first. `read_one` answers `None` on a release, autorepeat,
            // SYN or unmapped key, which re-polls.
            if let Some(buttons) = self.buttons.as_mut()
                && fds[1].revents & libc::POLLIN != 0
            {
                if let Some(page) = buttons.read_one()? {
                    return Ok(InputEvent::Page(page));
                }
                continue;
            }

            if fds[0].revents & libc::POLLIN != 0 {
                // `next_event` answers `None` short of a Down/Up boundary — a
                // move-only or partial packet — and re-polls. Touch is opened
                // `O_NONBLOCK` for this.
                if let Some(ev) = self.touch.next_event()? {
                    return Ok(InputEvent::Touch(ev));
                }
                continue;
            }

            // Spurious wake with no POLLIN — poll again.
        }
    }
}
