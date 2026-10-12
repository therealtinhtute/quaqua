# Project Planning Prompt: Coucou-like Media Island for Ubuntu GNOME

## Context

I want to build a Linux-native desktop application for **Ubuntu GNOME**, inspired by the UI/interaction concept of the GitHub project:

`https://github.com/Louis-CFM/coucou`

The goal is **not to port Coucou feature-for-feature**.

The target is a **Coucou-inspired desktop companion / Dynamic Island-style media control surface** for Ubuntu GNOME.

### Important scope decision

Ignore all **Claude Code / Claude Agent / Claude Hook / agent integration** concerns.

Do not spend planning effort on:
- Claude Code hooks
- Agent lifecycle
- PermissionRequest
- Agent sessions
- Agent event bus
- Anthropic API
- Agent orchestration

The project should instead focus on:

1. Ubuntu GNOME integration
2. Wayland compatibility
3. Top-bar / top-center island UI
4. Media player integration
5. MPRIS
6. Native Linux desktop behavior
7. Smooth animations and interaction
8. Architecture that can later support more desktop integrations

---

# Explicit Scope

The **v1 scope** is intentionally narrow.

### In scope

#### Platform
- Ubuntu GNOME
- Modern GNOME Shell
- Wayland-first design
- Linux desktop integration
- Single-user desktop environment

#### UI
- Top-center floating/island-style media UI
- Hidden / peek / compact / expanded states
- Hover and click interactions
- Play/pause/previous/next controls
- Track metadata
- Album artwork
- Playback progress
- Basic transitions and animations
- Keyboard accessibility where practical
- Multi-monitor behavior
- Fractional scaling considerations

#### Media
- MPRIS-compatible players
- MPRIS discovery
- Active-player selection
- Playback state
- Track metadata
- Artwork
- Position/duration where supported
- Playback controls
- Player changes / track changes
- Graceful handling when no player is active

#### Architecture
- Clear separation between:
  - media integration
  - domain/state model
  - UI state machine
  - GNOME/platform integration
- Event-driven updates where practical
- Adapter-based MPRIS integration
- Failure isolation between media integration and GNOME UI

#### Packaging / lifecycle
- Start automatically with the desktop
- Clean install/uninstall path
- Evaluate `.deb`, Flatpak, GNOME Extension packaging, and/or user services
- Basic logging and diagnostics

---

# Explicit Non-Goals

The following are **out of scope for v1** unless research shows they are required for the selected architecture.

### Agent / AI
- Claude Code integration
- Claude hooks
- AI agents
- LLM APIs
- agent orchestration
- coding assistant features
- permission approval UI for agents
- terminal monitoring

### Cross-platform support
Do not optimize the initial architecture for:

- macOS
- Windows
- KDE Plasma
- COSMIC
- XFCE
- other desktop environments

The architecture may expose clean platform boundaries, but **Ubuntu GNOME is the only required target for v1**.

### Media ecosystem expansion
Do not initially build:

- custom Spotify API integration
- YouTube API integration
- Apple Music API integration
- streaming-service authentication
- playlist management
- queue editing
- lyrics services
- recommendation systems
- cloud music synchronization

Use **MPRIS as the primary abstraction**.

### System-control expansion
Do not initially implement:

- volume mixer
- microphone control
- Bluetooth management
- Wi-Fi management
- battery dashboard
- notification center
- calendar
- system monitoring dashboard
- application launcher

These are future extension points only.

### Desktop shell replacement
Do not attempt to replace or heavily modify:

- GNOME Shell itself
- GNOME's full top bar
- system menus
- window manager behavior
- compositor behavior

The project should behave as a **small, focused desktop companion**, not a replacement shell.

### Advanced window/compositor hacks
Avoid depending on undocumented compositor behavior unless absolutely necessary.

Do not make assumptions that only work on:
- X11
- a specific Wayland compositor
- a specific Ubuntu patch
- one GNOME minor version

Any unavoidable platform-specific behavior must be explicitly documented.

### Visual over-engineering
Do not prioritize:
- complex 3D rendering
- large animation frameworks
- heavy visual effects
- GPU-heavy effects
- elaborate mascot systems

The first release should prioritize **native behavior, reliability, and interaction quality** over visual complexity.

### Backend complexity
Do not introduce a database, message broker, cloud backend, or multi-process architecture unless the selected design genuinely requires it.

Prefer the smallest architecture that satisfies the requirements.

---

# Scope Boundary Rule

When evaluating a proposed feature, classify it as:

```text
MUST HAVE
  → required for v1

NICE TO HAVE
  → useful but not necessary for v1

FUTURE
  → valid extension point, explicitly deferred

OUT OF SCOPE
  → do not design around it
```

Do not allow future integrations to distort the v1 architecture.

The architecture should be **extensible, but the implementation should remain small**.

---

# Definition of Done for v1

The project is considered successful when:

1. It runs reliably on Ubuntu GNOME under Wayland.
2. It detects an active MPRIS media player.
3. It displays the current track and artwork.
4. Play/pause/previous/next work.
5. The island appears, expands, and collapses smoothly.
6. Track changes update the UI without polling-heavy behavior.
7. No active player results in a clean hidden state.
8. Multi-monitor behavior is explicitly defined and tested.
9. GNOME remains usable when the application encounters an error.
10. The implementation does not require Claude Code, AI, or any external cloud service.

---

# Everything Below Must Respect This Scope

When producing the architecture proposal, technology comparison, repository structure, milestones, risks, and proof-of-concept:

- Do not re-introduce excluded features.
- Do not optimize for cross-platform support prematurely.
- Do not design infrastructure for future requirements unless it is cheap and clearly beneficial.
- Keep the first milestone focused on **Ubuntu GNOME + MPRIS + minimal island UI**.

The first architectural question to answer is:

> **What is the smallest reliable GNOME/Wayland architecture that can render a top-center media island and communicate with MPRIS?**

All subsequent design decisions should follow from that constraint.

---

# Goal

Create a **GNOME-native Dynamic Island / notch-like media control UI** similar in spirit to Coucou.

...