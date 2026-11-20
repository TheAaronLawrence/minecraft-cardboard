minecraft-cardboard {version} (preview, built from {commit})
A Rust rebuild of Minecraft: Java Edition, matching vanilla 26.3. For now it's a world viewer.

Run: open minecraft-cardboard-launcher.exe, which you download on its own from the
release. It installs the game into %APPDATA%\.minecraft-cardboard\game (this folder), with
Minecraft 26.3's textures and data downloaded from Mojang (about 41 MB, once), and each
time it opens it updates the game to the newest release, downloading just the new exe
unless the resources changed. Set your player name and press Play. The game opens on its
title screen: Singleplayer opens the bundled test world, and Multiplayer asks for a
server's address. Only offline-mode servers (online-mode=false in server.properties) work
so far.

In the window: click to capture the mouse, WASD to move, Space and Shift to go up and down,
Ctrl to fly faster, Esc to pause. On a server: Space jumps, Shift sneaks, Ctrl sprints, left
click mines, right click places, middle click picks a block, 1 to 9 or the wheel choose the
hotbar slot, E opens the inventory, T opens the chat and / opens it to type a command.

You can also start the game without the launcher. From PowerShell, in this folder:
  .\minecraft-cardboard.exe --pos -40,252,6 --look 180,10
      starts at the row of test blocks in the sky
  .\minecraft-cardboard.exe --world "$env:APPDATA\.minecraft\saves\My World"
      opens your own world; it must be saved by Minecraft 26.x
Other options: --distance CHUNKS, --fov DEG, --ticks N, --screenshot OUT.png --size WxH

Textures and models are Mojang's, from Minecraft 26.3.
