# rdev (Windows Dead-Key Fix Fork)

[![build](https://github.com/luisoncpp/rdev-deadkeys-fix/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/luisoncpp/rdev-deadkeys-fix/actions/workflows/rust.yml)
[![Crate](https://img.shields.io/crates/v/rdev.svg)](https://crates.io/crates/rdev)

Simple library to listen and send events **globally** to keyboard and mouse on macOS, Windows and Linux (x11).

## 🚀 Why this fork?

This version specifically fixes a critical issue in Windows regarding **dead-key consumption**.

In the original library, using the `listen` function on Windows would inadvertently clear the system's keyboard buffer. This caused:
- **Dead keys** (accents `´`, umlauts `¨`, circumflex `^`, backticks `` ` ``) to stop working system-wide while the application was running.
- International layouts (Spanish, French, German, etc.) were unable to type combined characters correctly in other applications.

**This fork implements the `TUE_NOCONSUME` (0x4) flag** in `ToUnicodeEx` calls, allowing the library to observe keyboard events without interfering with the global Windows keyboard state.

## 📦 Usage

To use this patched version in your Rust project, add the following to your `Cargo.toml`:

```toml
[dependencies]
rdev = { git = "[https://github.com/luisoncpp/rdev-deadkeys-fix.git](https://github.com/luisoncpp/rdev-deadkeys-fix.git)", branch = "master" }
```

---

## Listening to global events

```rust
use rdev::{listen, Event};

// This will block.
if let Err(error) = listen(callback) {
    println!("Error: {:?}", error)
}

fn callback(event: Event) {
    match event.name {
        Some(string) => println!("User wrote {:?}", string),
        None => (),
    }
}
```

### OS Caveats:

#### Windows (Fixed in this fork)
Unlike the original `rdev`, this fork **does not break** dead-key behavior. It is safe to use in "Always on Top" widgets or background idle games without disrupting the user's typing experience in other windows.

#### macOS
The process running the blocking `listen` function (loop) needs to be the parent process. Access to the Accessibility API must be granted in System Preferences.

#### Linux
The `listen` function uses X11 APIs and will not work in Wayland or the Linux kernel virtual console.

## Keyboard state

This fork significantly improves `KeyboardState` support for international layouts on Windows.

```rust
use rdev::{Keyboard, EventType, Key, KeyboardState};

let mut keyboard = Keyboard::new().unwrap();
let string = keyboard.add(&EventType::KeyPress(Key::BackQuote));
// With this fork, this now correctly resolves on Windows with LATAM/ES layouts.
```

## Main structs

### Event

`EventType` corresponds to a *physical* event.
`Event` corresponds to an actual event received, and `Event.name` reflects the character interpreted by the OS, respecting the active layout.

```rust
#[derive(Debug)]
pub struct Event {
    pub time: SystemTime,
    pub name: Option<String>,
    pub event_type: EventType,
}
```

## License
MIT
```
