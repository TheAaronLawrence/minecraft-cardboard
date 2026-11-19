minecraft-cardboard {version} (preview, built from {commit})
A Rust rebuild of Minecraft: Java Edition, matching vanilla 26.3. For now it's a world viewer.

Run: double-click minecraft-cardboard.exe. It opens the bundled test world.
Keep the exe next to the "resources" and "fixtures" folders.

Update: double-click update.cmd. It fetches the newest release into this folder, and
downloads just the new exe unless the game's resources changed. The repository is private,
so it needs your GitHub login: install the GitHub CLI (https://cli.github.com) and run
"gh auth login" once.

In the window: click to capture the mouse, WASD to move, Space and Shift to go up and down,
Ctrl to fly faster, Esc to release the mouse.

From PowerShell, in this folder:
  .\minecraft-cardboard.exe --pos -40,252,6 --look 180,10
      starts at the row of test blocks in the sky
  .\minecraft-cardboard.exe --world "$env:APPDATA\.minecraft\saves\My World"
      opens your own world; it must be saved by Minecraft 26.x
Other options: --distance CHUNKS, --fov DEG, --ticks N, --screenshot OUT.png --size WxH

Textures and models are Mojang's, from Minecraft 26.3.
