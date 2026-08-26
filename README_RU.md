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

- [ ] Основа приложения и MD3 theme
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

## 🔧 Разработка

TabletFlow пока только строится, поэтому нормальная инструкция появится после стабилизации структуры проекта.

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
