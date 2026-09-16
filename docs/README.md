# Spec Chum documentation

Guides for **using**, **building**, and **contributing** to Spec Chum today.
LLM assistant policy lives elsewhere (`AGENTS.md`, `.cursor/rules/`, `.cursor/skills/`) — not in these product docs.

**Public marketing site (GitHub Pages):** [www/](www/) — static landing (download / platforms). Deploys on **release publish** (not every `main` push). Setup / custom domain: [www/README.md](www/README.md).

## Players / end users

Start here if you want to run Spec Chum and play Spectrum software.

| Doc | What it covers |
| --- | --- |
| [../README.md](../README.md) | Product overview, quick start, releases |
| [ROMS.md](ROMS.md) | Where ROMs come from; what is / isn’t redistributed |
| [MACOS_NATIVE.md](MACOS_NATIVE.md) | Native macOS SpecChumMac app (build & use) |
| [WINDOWS_NATIVE.md](WINDOWS_NATIVE.md) | Optional Win32 shell (`windows_shell`) |
| [LINUX_NATIVE.md](LINUX_NATIVE.md) | Optional GTK4 shell (`linux_shell`); egui is Linux release primary |
| [LIVING_ROOM.md](LIVING_ROOM.md) | Experimental Bevy 3D CRT living-room mode |
| [MULTIFACE.md](MULTIFACE.md) | Multiface 1 attach / firmware notes |
| [TIMEX.md](TIMEX.md) | Timex TC2048 / TS2068 models and docks |
| [TAPE_IDENTITY.md](TAPE_IDENTITY.md) | How media titles / hashes are shown |

**Beginner path:** download a [GitHub Release](https://github.com/mward-sudo/spec_chum/releases), or build with `./scripts/fetch_roms.sh` then `cargo run -p app --release` (see root README).

**Advanced path:** native shells, living-room mode, Multiface / Timex / tape identity — linked above.

## Developers / contributors

Build, test, package, and extend the emulator.

| Doc | Audience | What it covers |
| --- | --- | --- |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Contributors | Rust practices, PRs, review gates, stack workflow |
| [TESTING.md](TESTING.md) | Contributors | Test tiers, lint inventory, accuracy vs convenience |
| [RELEASE.md](RELEASE.md) | Maintainers | Tagging `vX.Y.Z`, artifacts, slow-suite gate |
| [UI_ARCHITECTURE.md](UI_ARCHITECTURE.md) | Contributors | Host stack choices; native-shell strategy (#351) |
| [DEBUGGING.md](DEBUGGING.md) | Contributors | Trace categories, `spec_chum debug`, Inspect |
| [AGENT_DEBUG_API.md](AGENT_DEBUG_API.md) | Contributors / automation | Loopback HTTP control & inspect API |
| [../packaging/icon/README.md](../packaging/icon/README.md) | Packaging | Shared app icon assets |

## Automation surface (product feature)

The **Agent Debug HTTP API** (`spec_chum --serve`, `SPEC_CHUM_AGENT=1`) is a **developer automation** feature: localhost control, inspect, and 1:1 framebuffer PNG export. The word “agent” here means that HTTP surface / env vars — not LLM coding assistants.

Details: [AGENT_DEBUG_API.md](AGENT_DEBUG_API.md), [DEBUGGING.md](DEBUGGING.md).

## LLM assistant instructions (not product docs)

| Location | Role |
| --- | --- |
| [../AGENTS.md](../AGENTS.md) | Crate map, hard constraints, agent workflow |
| [../.cursor/rules/](../.cursor/rules/) | Always-on Cursor project rules |
| [../.cursor/skills/](../.cursor/skills/) | Task skills (e.g. emulator debugging) |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) § AI / agent-assisted work | Narrow contributor section for assisted PRs |
