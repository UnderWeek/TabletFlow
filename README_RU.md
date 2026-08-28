<div align="center">

# ✦ TabletFlow

### Modern tablet configuration, powered by OpenTabletDriver Daemon.

Современный **Material Design 3** frontend для [OpenTabletDriver](https://github.com/OpenTabletDriver/OpenTabletDriver).

[English](README.md) · **Русский**

</div>

---

> [!WARNING]
> **TabletFlow находится на ранней стадии разработки.**  
> Функции, интерфейс и совместимость могут сильно меняться.

## ✨ Что такое TabletFlow?

**TabletFlow** — независимое desktop-приложение для настройки планшетов через **OpenTabletDriver Daemon**.

OpenTabletDriver отвечает за работу с планшетом и обработку ввода.  
TabletFlow отвечает за **современный, красивый и понятный интерфейс**.

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
        Планшет
```

## 🎯 Цели

- 🎨 Интерфейс на **Material Design 3**
- 🍎 Нативная поддержка **Apple Silicon / ARM64**
- 🖥️ macOS, Windows и Linux
- 📐 Визуальный редактор области планшета
- 🖊️ Настройка пера и давления
- 🎛️ Удобные бинды кнопок
- 👤 Несколько профилей
- 🔌 Интеграция с плагинами OpenTabletDriver
- 🩺 Понятная диагностика

## 📐 Area Editor

Настройка области должна быть одновременно **визуальной и точной**.

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

Планируется:

- drag & resize
- точные размеры
- rotation
- centering
- full area
- блокировка aspect ratio
- mapping на монитор

## 🧩 Основные разделы

| Раздел | Для чего |
|---|---|
| 🏠 **Home** | Состояние планшета и daemon |
| 📐 **Area** | Область планшета и экрана |
| 🖊️ **Pen** | Перо и pressure |
| 🎛️ **Bindings** | Кнопки и сочетания клавиш |
| 👤 **Profiles** | Разные конфигурации |
| 🔌 **Plugins** | Плагины OpenTabletDriver |
| ⚙️ **Settings** | Настройки TabletFlow |
| 🩺 **Diagnostics** | Логи и диагностика |

## 🗺️ Roadmap

- [x] Основа приложения и MD3 theme (prototype)
- [ ] Подключение к OpenTabletDriver Daemon
- [ ] Обнаружение планшетов
- [ ] Чтение и применение конфигурации
- [ ] Tablet Area Editor
- [ ] Настройка мониторов
- [ ] Настройки пера
- [ ] Bindings
- [ ] Profiles
- [ ] Plugins
- [ ] Diagnostics
- [ ] macOS ARM64 release
- [ ] Сборки для Windows и Linux

## 💻 Платформы

| Платформа | Архитектура | Цель |
|---|---|---|
| macOS | ARM64 | ⭐ Основная |
| macOS | x86-64 | Планируется |
| Windows | x86-64 | Планируется |
| Linux | x86-64 | Планируется |

Поддержка конкретных планшетов в первую очередь зависит от **OpenTabletDriver**.

##  Установка macOS ARM64

У TabletFlow есть нативная **ARM64**-сборка для Mac с Apple Silicon (M1, M2, M3 и M4).

1. Скачайте `TabletFlow-<version>-macos-arm64.dmg` из GitHub Release.
2. Откройте DMG и перетащите `TabletFlow.app` в папку `Applications`.
3. Откройте Terminal и снимите quarantine с загруженного приложения:

   ```bash
   xattr -dr com.apple.quarantine "/Applications/TabletFlow.app"
   ```

4. Запустите TabletFlow из Applications или командой:

   ```bash
   open "/Applications/TabletFlow.app"
   ```

OpenTabletDriver daemon уже находится внутри приложения и запускается вместе с TabletFlow.

Если macOS пишет, что TabletFlow повреждён, это означает блокировку Gatekeeper, а не повреждение файла. Повторите команду `xattr`, затем нажмите правой кнопкой по `TabletFlow.app` в Applications и выберите **Open**.

Текущая ARM64-сборка распространяется без Apple Developer-подписи, поэтому такое одноразовое разрешение ожидаемо. Если macOS запросит разрешения для планшета, выдайте их в разделе **Системные настройки → Конфиденциальность и безопасность**.

## 🔧 Разработка

Первый прототип использует Rust и Slint. Интерфейс компилируется в нативный бинарник — без WebView и JavaScript runtime.

```bash
cargo run
cargo build --release
```

Сейчас в прототипе есть лёгкий каркас приложения, компактная навигация и честные empty states. Данные планшета, профиля и daemon остаются пустыми, пока их не передаст backend OpenTabletDriver.

```bash
git clone https://github.com/UnderWeek/TabletFlow.git
cd TabletFlow
```

## 🤝 OpenTabletDriver

TabletFlow **не является официальным проектом OpenTabletDriver**.

Это независимый frontend, использующий OpenTabletDriver Daemon в качестве backend.

Огромное спасибо разработчикам и contributors [OpenTabletDriver](https://github.com/OpenTabletDriver/OpenTabletDriver) за драйверную часть и поддержку огромного количества планшетов.

## 📜 Лицензия

Смотрите [`LICENSE`](LICENSE).

OpenTabletDriver и другие сторонние компоненты распространяются на условиях собственных лицензий.

---

<div align="center">

### ✦ TabletFlow

**Настрой планшет. Вернись в свой flow.**

</div>
