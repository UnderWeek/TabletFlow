<div align="center">

# ✦ TabletFlow

### Modern tablet configuration, powered by OpenTabletDriver Daemon.

A focused native desktop frontend for [OpenTabletDriver](https://github.com/OpenTabletDriver/OpenTabletDriver).

**English** · [Русский](README_RU.md)

</div>

---

## ✨ What is TabletFlow?

**TabletFlow** is an independent desktop frontend for **OpenTabletDriver Daemon**.

OpenTabletDriver handles the tablet and input processing.  
TabletFlow provides a clear overview, precise active-area editing and dependable application preferences.

```text
┌──────────────────────┐
│      TabletFlow      │
│      Native UI       │
└──────────┬───────────┘
           │
           │ IPC
           ▼
┌──────────────────────┐
│ OpenTabletDriver     │
│ Daemon               │
└──────────┬───────────┘
           │
           ▼
      Graphics Tablet
```

## 🎯 Current scope

- 🏠 **Overview** with OpenTabletDriver and connected-tablet status
- 📐 **Active area** with a visual draft, exact values and inline input checks
- ✅ Applying format-checked area changes and confirming the values read back from OpenTabletDriver
- ⚙️ **Settings** for appearance, startup and background behavior
- 🔌 IPC connection, tablet detection and configuration reading

## 📐 Active area editor

Active-area configuration is visual **and** precise. TabletFlow reads the current values from OpenTabletDriver and keeps edits in a local draft until they are valid and ready to apply.

```text
┌─────────────────────────────────┐
│ Tablet Area                     │
│                                 │
│   ┌─────────────────────────┐   │
│   │      ┌────────────┐     │   │
│   │      │ Active Area│     │   │
│   │      └────────────┘     │   │
│   └─────────────────────────┘   │
│                                 │
│  Width   Height   X   Y   Rot.  │
└─────────────────────────────────┘
```

The editor currently provides:

- exact width, height, X, Y and rotation values
- millimetre and degree units
- inline input checks before Apply becomes available
- a preview driven by the current draft
- Revert for discarding local changes
- confirmation after OpenTabletDriver reports the applied values

## 🧩 Available pages

| Page | Purpose |
|---|---|
| 🏠 **Overview** | Connection and connected-tablet status |
| 📐 **Active area** | Review, check and apply tablet-area values |
| ⚙️ **Settings** | Appearance and application behavior |

## 🗺️ Roadmap

- [x] Native application shell and responsive interface
- [x] Connect to OpenTabletDriver Daemon over IPC
- [x] Detect connected tablets
- [x] Read the current active-area configuration
- [x] Check draft values and apply active-area changes
- [x] Confirm applied values from OpenTabletDriver
- [ ] Direct drag and resize in the area preview
- [ ] Center, full-area and aspect-ratio controls
- [ ] Display mapping
- [ ] Pen configuration
- [ ] Bindings
- [ ] Profiles
- [ ] Plugins
- [ ] Diagnostics
- [x] macOS ARM64 release
- [ ] Windows & Linux builds

## 💻 Platforms

| Platform | Architecture | Target |
|---|---|---|
| macOS | ARM64 | ⭐ Primary |
| macOS | x86-64 | Planned |
| Windows | x86-64 | Planned |
| Linux | x86-64 | Planned |

Hardware compatibility is primarily provided by **OpenTabletDriver**.

##  macOS ARM64 installation

TabletFlow has a native **ARM64** build for Apple Silicon Macs (M1, M2, M3 and M4).

1. Download `TabletFlow-<version>-macos-arm64.dmg` from the GitHub release.
2. Open the DMG and drag `TabletFlow.app` to `Applications`.
3. Open Terminal and remove the download quarantine:

   ```bash
   xattr -dr com.apple.quarantine "/Applications/TabletFlow.app"
   ```

4. Start TabletFlow from Applications or with:

   ```bash
   open "/Applications/TabletFlow.app"
   ```

The OpenTabletDriver daemon is bundled inside the application and starts with TabletFlow.

If macOS says that TabletFlow is damaged, the application is blocked by Gatekeeper rather than actually corrupted. Repeat the `xattr` command, then right-click `TabletFlow.app` in Applications and choose **Open**.

The current ARM64 build is distributed without an Apple Developer signature, so this one-time approval is expected. If macOS requests tablet permissions, grant them in **System Settings → Privacy & Security**.

## 🔧 Development

TabletFlow uses Rust and Slint. The UI is compiled into the native binary, with no WebView or JavaScript runtime.

```bash
cargo run
cargo build --release
```

The current application includes Overview, format-checked draft editing for the active area, and Settings. Device and area values remain unavailable until they are supplied by the OpenTabletDriver backend.

```bash
git clone https://github.com/UnderWeek/TabletFlow.git
cd TabletFlow
```

## 🤝 OpenTabletDriver

TabletFlow is **not an official OpenTabletDriver project**.

It is an independent frontend built around the OpenTabletDriver Daemon.

Huge thanks to the [OpenTabletDriver](https://github.com/OpenTabletDriver/OpenTabletDriver) maintainers and contributors for making the backend and tablet support possible.

## 📜 License

See [`LICENSE`](LICENSE).

Third-party software, including OpenTabletDriver, is covered by its own license.

---

<div align="center">

### ✦ TabletFlow

**Configure your tablet. Get back into the flow.**

</div>
