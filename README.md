<img src="assets/icon.svg" width="64" height="64" alt="">

# CF File Manager

A small desktop file manager written in Rust with [egui](https://www.egui.rs/). It builds natively for Windows and Linux.

## Features

- Browse folders from the sidebar, address bar, back/forward history, and parent-folder button.
- Pin frequently used folders in the sidebar; pinned folders are saved between sessions.
- Drag items onto folder rows to move them, or drop files and folders from the desktop onto a folder to copy them there. On Windows, dragging items to another app copies them and leaves the original in place.
- The Downloads folder opens sorted by modified time, newest first.
- Search the current folder and toggle dot-prefixed hidden files.
- View file type, size, and modified time; open files with the system's default app.
- Create folders, rename items, and delete files or folders after confirmation.
- Keyboard shortcuts: `Alt+Left` / `Alt+Right` for history, `Ctrl+L` for the location bar, `Ctrl+R` to refresh, `Ctrl+Shift+N` for a new folder, and `Delete` to start a confirmed delete.

## Build and run

Install the stable Rust toolchain first, then clone this repository:

```sh
git clone https://github.com/classicfoo/cf-file-manager.git
cd cf-file-manager
```

Cargo fetches `egui-classic95` from its `main` branch, so the app does not need a second repository beside it. `Cargo.lock` records the exact library commit used by normal builds.

To refresh the app to the latest library commit, run:

```sh
cargo update -p egui-classic95
cargo build --locked --release
```

Commit and push the updated `Cargo.lock` with the app changes. Other computers then get that same library revision with `git pull` and `cargo run --locked --release`.

On Linux, install the native windowing and OpenGL build dependencies:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libwayland-dev libegl1-mesa-dev libgles2-mesa-dev pkg-config
```

Then build or run the app:

```sh
cargo run --locked --release
```

On Windows, use the Rust MSVC toolchain and Visual Studio C++ Build Tools. After updating an existing checkout, run:

```powershell
git pull
cargo run --locked --release
```

The GitHub Actions workflow builds both Windows and Linux release binaries on every push and pull request.
