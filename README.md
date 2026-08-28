<div align="center">

# ✦ TabletFlow

### Modern tablet configuration, powered by OpenTabletDriver Daemon.

A modern **Material Design 3** frontend for [OpenTabletDriver](https://github.com/OpenTabletDriver/OpenTabletDriver).

**English** · [Русский](README_RU.md)

</div>

---

> [!WARNING]
> **TabletFlow is currently in early development.**  
> Features, UI and compatibility may change at any time.

## ✨ What is TabletFlow?

**TabletFlow** is an independent desktop frontend for **OpenTabletDriver Daemon**.

OpenTabletDriver handles the tablet and input processing.  
TabletFlow focuses on making configuration **modern, visual and pleasant to use**.

```text
┌──────────────────────┐
│      TabletFlow      │
│       MD3 UI         │
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

## 🎯 Goals

- 🎨 **Material Design 3** interface
- 🍎 Native **Apple Silicon / ARM64** support
- 🖥️ macOS, Windows and Linux
- 📐 Visual tablet area editor
- 🖊️ Pen and pressure configuration
- 🎛️ Easy button bindings
- 👤 Multiple configuration profiles
- 🔌 OpenTabletDriver plugin integration
- 🩺 Useful diagnostics

## 📐 Area Editor

Tablet area configuration should be visual **and** precise.

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

Planned controls include:

- drag & resize
- exact dimensions
- rotation
- centering
- full area
- aspect-ratio lock
- display mapping

## 🧩 Planned Pages

| Page | Purpose |
|---|---|
| 🏠 **Home** | Device and daemon status |
| 📐 **Area** | Tablet and display mapping |
| 🖊️ **Pen** | Pressure and pen settings |
| 🎛️ **Bindings** | Buttons and shortcuts |
| 👤 **Profiles** | Multiple configurations |
| 🔌 **Plugins** | OpenTabletDriver plugins |
| ⚙️ **Settings** | TabletFlow preferences |
| 🩺 **Diagnostics** | Logs and connection state |

## 🗺️ Roadmap

- [x] Application shell & MD3 theme (prototype)
- [ ] Connect to OpenTabletDriver Daemon
- [ ] Detect connected tablets
- [ ] Read and apply configuration
- [ ] Tablet Area editor
- [ ] Display mapping
- [ ] Pen configuration
- [ ] Bindings
- [ ] Profiles
- [ ] Plugins
- [ ] Diagnostics
- [ ] macOS ARM64 release
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

The first prototype uses Rust and Slint. The UI is compiled into the native binary, with no WebView or JavaScript runtime.

```bash
cargo run
cargo build --release
```

The current prototype contains a lightweight application shell, compact navigation and honest empty states. Device, profile and daemon values remain empty until they are supplied by the OpenTabletDriver backend.

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
