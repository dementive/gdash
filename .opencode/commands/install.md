---
description: Build release and install gdash so it can be tested from the Start Menu
---

Install the current build for real-machine testing: from the repo root, run
`powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 $ARGUMENTS`
in the background (it builds the release exe, copies it to
`%LOCALAPPDATA%\Programs\gdash`, and refreshes the Start Menu shortcut).
Report the result when it completes. If the script fails because gdash is
running, tell me to exit it first (guide menu > Exit gdash) rather than
killing the process.
