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

- [ ] Application shell & MD3 theme
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

## 🔧 Development

TabletFlow is still being built, so development instructions will be added once the project structure stabilizes.

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
