# UMBRA Hybrid

UMBRA is an experimental, source-only Windows x64 fork of VIctory for Xbox Series compatibility. It runs the title from files you supply and translates RDNA2 GPU programs to DXIL for Direct3D 12. Use your own copy of the title. The project is unaffiliated with its developer and publisher.

## What's working so far

- The launcher starts the title and offers shader precompilation, on-demand caching, and a cache-off mode. The latest cache implementation validates saved entries and recompiles damaged ones. In a local warm-cache run before that revision, the launcher reused **25,741 translated shaders**. Direct3D 12 pipeline objects are a separate cache.
- The opening cinematic and first playable scenes render, including character geometry and many world textures. The runtime includes fixes for mesh and pixel shaders, indirect work, streamed texture views, and intro-frame uploads.
- The runtime includes changes to CPU-side buffer refresh, texture preparation, and GPU copy-back. The latest source includes parallel staging for independent textures and preserves an already displayed texture during a transient mapping miss. The performance and visual impact of those newest changes is still unverified in-game.

## Local results and limits

On a Ryzen 5 5600X, Radeon RX 9060 XT, and 32 GB RAM using the Series X profile, steadier portions of one gameplay run took about **320–344 ms per frame (roughly 3 FPS)**. Texture streaming occasionally pushed frame times substantially higher. These are development measurements on one PC.

Rendering remains incomplete. Textures can flicker or arrive late, and material, lighting and depth issues remain. A later warm-cache run ended with DXGI device-hung error `0x887A0006`. Its cause is still under investigation. The default profile keeps ray tracing off. `-Resolution 720p` requests a display mode. The title can still render from a 4K source. This is a development snapshot.

## Build and launch

Use Windows x64, Rust with the pinned toolchain, the MSVC C++ build tools, Python 3 with Tk, and a Direct3D 12 GPU with Shader Model 6.6 and mesh shader support.

```powershell
cargo build --release -p victory
.\RUN_UMBRA_HYBRID.ps1 -GpuSchedule safe -CacheMode warm
```

Run these from the repository root. The launcher can prompt for your executable and remember its path locally. Pass `-GameExe` to supply it directly. `-CacheMode lazy` builds the cache during play.

## Upstream and contributors

- [Marvisak/VIctory](https://github.com/Marvisak/VIctory) is the base for the loader, runtime services, Direct3D 12 backend and RDNA2 shader translator. Credit goes to Marvisak and the VIctory contributors.
- [JoshuuDev/victory-amd](https://github.com/joshuudev/victory-amd), [Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD), and [DexNom/VIctory](https://github.com/DexNom/VIctory) contributed the AMD-oriented patch series used in the Hybrid integration. See the [integration notes](HYBRID_REPAIR_NOTES.txt) for what was ported.
- [vitutu1/victory-perf](https://github.com/vitutu1/victory-perf) supplied the GPU snapshot hash approach and the reference for scoped indirect-work barriers.
- [xcl-dev/xcl](https://github.com/xcl-dev/xcl), originally by ivall and contributors, was a reference for runtime behavior. The corresponding changes were implemented in VIctory.
- The UMBRA integration was AI-assisted. The base fork credits Claude Code, and the Hybrid work was also developed with OpenAI Codex.

UMBRA and its VIctory base are licensed under GPL-3.0-or-later. See [LICENSE](LICENSE) and the [asset policy](docs/asset-policy.md). Game files, images and generated caches stay local.

## Upstream VIctory documentation

The following section documents upstream VIctory and its earlier status.

---

# VIctory

Runs the title (Xbox Series X|S) natively on Windows, from files
the user supplies.

The title's x86-64 code runs unchanged inside our process; VIctory supplies
the platform underneath it and converts the title's data, ahead of time where
it can and on first use where it must (shaders). See
[docs/architecture.md](docs/architecture.md).

## Status (2026-11-23)

**Not playable yet.** The title boots, runs its menus at its own 30 fps and
loads the world, where rendering fails.

| Stage | State |
|-------|-------|
| Boot, first-boot screens (accessibility, language, settings, legal) | works, 30 fps |
| Intro video with sound | works |
| Start screen, profile and saves | works |
| Loading into the world (about 50 s) | works |
| The world (first mission's intro) | reached and runs with indirect draws; its title card renders, the scene after it is still dark; 0.5-1.8 s per frame |

Tested on one machine: Windows 11, AMD RX 6700 XT.

> **Warning.** Entering the world on a real GPU can currently hang it, and
> Windows then resets the display driver (every monitor goes black for a
> moment). To look further, use a diagnostics build with
> `VICTORY_D3D12_WARP=1`, which renders on the software adapter.

What each part does today and what it still needs:

| Subsystem | Today | Needs |
|-----------|-------|-------|
| Loader | Runs the title's executable and its DLLs in our process; crash reports name the faulting code and are kept in `crashes.log` | – |
| Runtime services | One signed-in offline user, licences, packages, saves that survive a kill at any step, text entry dialog | Speech is silent; the dialog and relaunch are untested live |
| Network | Offline by design: the title cannot reach the network | – |
| Files and streaming | Archives and streamed data read, zlib and BCPack decoded in parallel | Loading is limited by writes into title memory |
| Input | XInput pads, keyboard, keyboard stand-in pad | Trigger rumble |
| Audio | Menu, video and loading sound; XMA voices and Opus streams decode | Reverb is silent; Opus results unverified against the title |
| GPU: menus | Draws, compute and presentation on host D3D12; only changed title memory is copied | Frame pacing drifts on one screen in release builds |
| GPU: world | Mesh pipelines, shader-written buffers, descriptor lifetimes, indirect execution (arguments clamped), pipelines made by worker threads and kept on disk | Most of the scene is still missing (untranslated shaders drop draws); frames take seconds; real queries, depth bounds, HDR and scaling are missing |
| Shaders | Console GPU code recompiled to DXIL at first use, cached on disk. Menus: all. World: mesh 8,191 of 8,506, vertex/pixel 10,617 of 12,346, compute 1,643 of 2,539 | 64-bit image atomics, merged geometry stages, the ray-tracing builder's shaders; some compute programs need a GPU with 64-lane waves |
| Textures | Console tiled layouts and detiling for the modes the title uses | Render-target metadata planes |
| Series S mode | Runs through the menus to the loading screen | The world, as above |

## Getting started

Requirements: Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with
shader model 6.6 (and mesh shaders for the world), Rust (pinned in
`rust-toolchain.toml`).

The title also needs several GiB of available Windows commit at startup.
Check **Committed** under **Task Manager → Performance → Memory**. If
`crashes.log` reports Windows error 1455, close other applications or increase
the Windows paging file size, then retry.

```powershell
cargo xtask install-hooks                # asset-policy check on every commit
copy victory.example.toml victory.toml   # then set your paths (outside the repo)
cargo test --workspace                   # needs no game data
cargo run -p victory -- config           # check your paths
```

Run the title:

```powershell
cargo build --release -p victory
target\release\victory.exe run <game dir>\<the title's exe> --execute
```

The window caption shows the framerate. Saves, settings and `crashes.log`
are kept under the data folder (default `%LOCALAPPDATA%\victory\data`), derived
data under the cache folder.

Input: XInput pads. With none connected the keyboard is a pad while the
title's window has focus:

| Keys | Pad |
|------|-----|
| W A S D / I J K L | left / right stick |
| arrows | D-pad |
| Enter, Backspace, X, Y | A, B, X, Y |
| Q / E, Z / C | shoulders, triggers |
| F / H | stick clicks |
| Esc, Tab | Menu, View |

## Commands

```
victory inspect <exe|dll> [--imports] [--exports]
victory census [paths...] [--missing] [--strict]
victory lib-exports <import.lib> [--list]
victory shader-info [paths...] [--verbose] [--disasm]
victory translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
victory run <exe> [--execute] [--unimplemented trap|zero] [--strict]
            [--console series-x|series-s] [-- <title arguments>]
victory config
```

Logging: `VICTORY_LOG=victory_loader=trace,info` (a release build logs
info and above).

## Debugging

A release build has no debugging aids. Build them in with

```powershell
cargo build --profile diag -p victory --features diagnostics   # target\diag\victory.exe
```

| Variable | Effect |
|----------|--------|
| `VICTORY_LOG=<target>=debug,...` | debug and trace output |
| `VICTORY_STACKS=20,60` | every thread's stack at those seconds |
| `VICTORY_WATCH=<rva>,...` | logs a title u32 when it changes |
| `VICTORY_KEYS=30:Enter,35:X` | presses keys at those seconds, focus or not |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | saves title memory to the cache folder on a crash |
| `VICTORY_CAPTURE=<dir>` | saves presented frames (powers of two, or `VICTORY_CAPTURE_FRAMES=<list>`) |
| `VICTORY_DUMP_TARGETS=1` | with the two above, saves every render target of those frames |
| `VICTORY_D3D12_DEBUG=1` | host debug layer |
| `VICTORY_D3D12_GBV=1` | with the above, GPU-based validation (bad descriptor use reported, not faulted; slow) |
| `VICTORY_D3D12_DRED=1` | on a GPU failure, the allocation hit and the commands in flight |
| `VICTORY_PIPELINE_WAIT_MS=24` | reduce the per-frame wait for background pipeline creation from the default of 500 ms. A draw can be omitted until its pipeline is ready |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | omit draws and dispatches, including indirect ones, whose pipeline contains a listed 16-digit shader hash |
| `VICTORY_GPU_TIMING=1` | with the above, each list of a submission run and timed on its own; slow lists logged with their slowest commands |
| `VICTORY_D3D12_WARP=1` | host device on the software adapter (slow; cannot reset the driver) |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | translated shader loops stop after `n` iterations per lane instead of 2^20 (own shader cache entries); if a lower `n` makes world frames much faster, loops hitting the limit are what makes them slow |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | records what the title plays (our process only) |
| `VICTORY_XMA_DUMP=<dir>` | saves the first 32 XMA voices' context and input |

Every build appends crashes and host GPU failures to `crashes.log` in the
title's data folder.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
