<img src="assets/icon.svg" width="64" height="64" alt="">

# CF File Manager

A small desktop file manager written in Rust with [egui](https://www.egui.rs/). It builds natively for Windows and Linux.

## Features

- Browse folders from the sidebar, address bar, back/forward history, and parent-folder button.
- Pin frequently used folders in the sidebar; pinned folders are saved between sessions.
- Drag items onto folder rows to move them, or drop files and folders from the desktop to copy them into the open folder. On Windows, Ctrl+drag an item to copy it into another app.
- The Downloads folder opens sorted by modified time, newest first.
- Search the current folder and toggle dot-prefixed hidden files.
- View file type, size, and modified time; open files with the system's default app.
- Create folders, rename items, and delete files or folders after confirmation.
- Keyboard shortcuts: `Alt+Left` / `Alt+Right` for history, `Ctrl+L` for the location bar, `Ctrl+R` to refresh, `Ctrl+Shift+N` for a new folder, and `Delete` to start a confirmed delete.

## Build and run

Install the stable Rust toolchain first. On Linux, install the native windowing and OpenGL build dependencies:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libwayland-dev libegl1-mesa-dev libgles2-mesa-dev pkg-config
```

Then build or run the app:

```sh
cargo run --release
```

On Windows, build from a terminal with the Rust MSVC toolchain and Visual Studio C++ Build Tools installed:

```powershell
cargo run --release
```

The GitHub Actions workflow builds both Windows and Linux release binaries on every push and pull request.
