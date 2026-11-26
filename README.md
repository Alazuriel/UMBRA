# UMBRA

UMBRA is a Windows compatibility layer based on [VIctory](https://github.com/Marvisak/VIctory). To run it, supply your own Xbox Series game files.

## Performance on the UMBRA test PC

Measured on 26 November 2026 with the local `codex/umbra-context-glyphs` development build. The public `main` branch contains an earlier source snapshot.

| Component | Test system |
|---|---|
| CPU | AMD Ryzen 5 5600X, 6 cores / 12 threads |
| GPU | AMD Radeon RX 9060 XT, Windows driver 32.0.31044.16 |
| Memory | 32 GiB installed, 2 × 16 GiB at 3600 MT/s |
| OS | Windows 11 Pro, build 26300 |

| Scene | Observed performance |
|---|---|
| Boat, camera stationary | About 15–17 FPS. One HUD reading showed 16.0 FPS, 64.4 ms/frame and 4,282 draws/frame. |
| Prison gameplay | About 12.5–13.8 FPS in the sampled section. |
| Cinematic | About 6–7 FPS observed by the player. |
| Brief gameplay peaks | The player saw 18–19 FPS. |
| Intro and transition into a heavier scene | The log showed 30 FPS at 12 draws/frame, followed by a 1.5 FPS reporting interval at 867 draws/frame. |

These are scene-specific HUD readings and log intervals. A full-run average, 1% low and controlled before/after comparison are still unavailable. The boat HUD showed 99% usage on the busiest GPU engine and 30.28 of 31.89 GiB of system RAM in use. Distant foliage can disappear and reappear, textures can arrive late, and streaming transitions can still cause long stalls.

The development build draws on the [VIctory Belobug AMD fork](https://github.com/Belobug/VIctory-Belobug-AMD). Its published RX 7900 XTX figures were measured on a different PC. The UMBRA measurements above are from the RX 9060 XT.

## Running the public snapshot

Follow the [Hungarian build and launch guide](BUILD_AND_RUN_HU.txt) for the version available on this branch.

## Credits

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors is the base of everything here: the loader, the runtime
  services, the Direct3D 12 back end and the shader recompiler. Licensed
  GPL-3.0-or-later; so is this fork. 
- **[XCL](https://github.com/ivall/xcl)** by ivall and its contributors
  (GPL-2.0-only) was used as a reference for how the same title behaves under
  another layer. Fixes inspired by it were written anew in VIctory's code.
- This fork's changes were developed with [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
