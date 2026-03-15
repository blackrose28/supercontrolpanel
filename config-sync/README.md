## Config Sync for SuperControlPanel

This folder is a **template** for a private Git repo that keeps your SuperControlPanel config in sync across devices.

Recommended use: create a **new private repo** on your Git host (GitHub/GitLab/Codeberg/etc.) named something like `config-sync`, then copy this structure into it (or clone this project and move this folder into that repo).

The app itself still reads/writes `config.json` and `servers.json` in the OS-specific app config directory; this repo is the **shared source of truth** that the app’s Config page can sync against.

### Layout

- `app-config/config.json` – shared app config for SuperControlPanel
- `app-config/servers.json` – shared server inventory (optional but recommended)
- `scripts/sync-app-config.ps1` – Windows PowerShell helper (manual/CLI workflow)
- `scripts/sync-app-config.sh` – Linux/macOS shell helper (manual/CLI workflow)

You can add other app configs beside `app-config/` if you want this repo to manage multiple tools.

---

## 1. Create the private Git repo

1. On your Git host, create a **private** repo, e.g. `config-sync`.
2. On each device, clone it to a stable path, for example:
   - Windows: `D:\config-sync`
   - Linux/macOS: `~/config-sync`
3. Copy this `config-sync` folder’s contents into that repo (or move the entire folder and commit it).

After that, treat the cloned repo as the canonical location of:

- `app-config/config.json`
- `app-config/servers.json`

---

## 2. Wire SuperControlPanel to the repo (preferred: in-app UI)

SuperControlPanel writes its `config.json` / `servers.json` into the OS config directory (via Tauri’s app config path).

The **recommended** way to sync is to use the **Config page → “Config sync (Git repo)”** section:

- Set **Config repo root** to the local clone path on *this device*  
  (e.g. `D:\config-sync` or `~/config-sync`).
- Use the buttons:
  - **Copy from repo → app**: copies `app-config/config.json` and `app-config/servers.json` (when present) into the app config directory.
  - **Copy from app → repo**: copies the app config directory’s `config.json` and `servers.json` into `app-config/config.json` / `app-config/servers.json`.

Notes:

- The **Config repo root path is per-device** and stored in a local-only `sync-local.json` file under the app config directory. It is *not* synced via Git.
- Only `config.json` + `servers.json` are meant to be shared; secrets stay in the OS keyring.

---

## 3. Optional: Symlink or script-based wiring

If you prefer, you can still wire things manually using symlinks or the provided scripts.

### Option A: Symlink

Once you know the app config directory on a device, you can symlink the files:

- **Windows (PowerShell, elevated if needed)**:

```powershell
$appConfigDir   = "C:\Path\To\SuperControlPanel"
$repoConfigPath = "D:\config-sync\app-config\config.json"
$repoServersPath = "D:\config-sync\app-config\servers.json"

New-Item -ItemType Directory -Path $appConfigDir -Force | Out-Null

Remove-Item -Path (Join-Path $appConfigDir "config.json") -ErrorAction SilentlyContinue
New-Item -ItemType SymbolicLink -Path (Join-Path $appConfigDir "config.json") -Target $repoConfigPath

if (Test-Path $repoServersPath) {
  Remove-Item -Path (Join-Path $appConfigDir "servers.json") -ErrorAction SilentlyContinue
  New-Item -ItemType SymbolicLink -Path (Join-Path $appConfigDir "servers.json") -Target $repoServersPath
}
```

- **Linux/macOS**:

```bash
app_config_dir="$HOME/.config/SuperControlPanel"   # adjust to your actual path
repo_root="$HOME/config-sync"

mkdir -p "$app_config_dir"

ln -sf "$repo_root/app-config/config.json" "$app_config_dir/config.json"

if [ -f "$repo_root/app-config/servers.json" ]; then
  ln -sf "$repo_root/app-config/servers.json" "$app_config_dir/servers.json"
fi
```

With symlinks, editing settings or servers in the app writes directly into the repo checkout. You then `git commit` and `git push` those changes like normal.

### Option B: Sync scripts (manual CLI)

If you prefer not to use symlinks, use the scripts instead:

- `sync-app-config.ps1` on Windows
- `sync-app-config.sh` on Linux/macOS

Both support:

- **`pull`** – copy from repo → app config dir
- **`push`** – copy from app config dir → repo

They operate on `config.json` and can be extended to include `servers.json` if needed, but the in-app UI already handles both.

---

## 4. Git workflow

On a device where you update settings or servers:

1. `git pull` to ensure you have the latest state.
2. In the app, click **Copy from repo → app** (if you expect remote changes) before editing.
3. Change config and/or servers in the UI.
4. Click **Copy from app → repo** to stage updates into `app-config/`.
5. From a shell:
   - `git status` and verify `app-config/config.json` / `app-config/servers.json` changed.
   - `git commit -am "Update SuperControlPanel config"` (or similar).
   - `git push`.

On another device before using the app:

1. `git pull`.
2. In the app’s Config page, ensure **Config repo root** points to your clone on that device.
3. Click **Copy from repo → app**.
4. Launch SuperControlPanel – it now uses the shared config + servers.

---

## 5. Optional: backups via Google Drive or a server

For extra durability:

- **Google Drive/Dropbox/OneDrive**  
  - Clone your Git repo *inside* a cloud-synced folder, or periodically export snapshots (e.g. zip archives of the repo) into that folder.
  - This provides an additional offsite backup and a fallback in case your Git host is temporarily unavailable.

- **Personal server/VPS**  
  - Keep a clone of this repo on your server.
  - Optionally set up a cron job/systemd timer that periodically runs `git fetch` and archives checkpoints (e.g., tarballs).

Git already gives you history and rollback; these options mainly protect against losing access to your Git host or local machines.

---

## 6. Secrets

SuperControlPanel stores sensitive data (like passwords) in the OS keyring, **not** in `config.json` or `servers.json`.  
You should **keep that behavior** and avoid placing secrets directly in this repo.

If you later add other tools’ configs here, follow the same pattern:

- Keep secrets in OS keyring, password manager, or .env files that are **not** committed.
- Only sync non-sensitive configuration via Git.

## Config Sync for SuperControlPanel

This folder is a **template** for a private Git repo that keeps your SuperControlPanel config in sync across devices.

Recommended use: create a **new private repo** on your Git host (GitHub/GitLab/Codeberg/etc.) named something like `config-sync`, then copy this structure into it (or clone this project and move this folder into that repo).

The app itself still reads/writes `config.json` in the OS-specific config directory; this repo is the shared source of truth.

### Layout

- `app-config/config.json` – the shared config file for SuperControlPanel
- `scripts/sync-app-config.ps1` – Windows PowerShell helper
- `scripts/sync-app-config.sh` – Linux/macOS shell helper

You can add other app configs beside `app-config/` if you want this repo to manage multiple tools.

---

## 1. Create the private Git repo

1. On your Git host, create a **private** repo, e.g. `config-sync`.
2. On each device, clone it to a stable path, for example:
   - Windows: `D:\config-sync`
   - Linux/macOS: `~/config-sync`
3. Copy this `config-sync` folder’s contents into that repo (or move the entire folder and commit it).

After that, treat the cloned repo as the canonical location of `app-config/config.json`.

---

## 2. Wire SuperControlPanel to the repo

SuperControlPanel writes its `config.json` into the OS config directory (via Tauri’s app config path). To make it portable, you have two options per device:

- **Option A – Symlink (simple if supported)**  
  Point the app’s expected `config.json` to the repo’s `app-config/config.json`.

- **Option B – Sync scripts (more explicit, always works)**  
  Use the provided scripts to copy between the app config path and the repo before/after you use the app.

Because the exact config directory path is OS- and installation-dependent, you should locate it once on each machine (for example, by searching for `config.json` after saving settings in the app), then reuse that path in scripts or symlinks.

### Option A: Symlink

Once you know the app config path, you can create a symlink so the app reads directly from the repo file:

- **Windows (PowerShell, elevated if needed)**:

```powershell
$appConfigPath = "C:\Path\To\SuperControlPanel\config.json"
$repoConfigPath = "D:\config-sync\app-config\config.json"

Remove-Item -Path $appConfigPath -ErrorAction SilentlyContinue
New-Item -ItemType SymbolicLink -Path $appConfigPath -Target $repoConfigPath
```

- **Linux/macOS**:

```bash
app_config_path="$HOME/.config/SuperControlPanel/config.json"   # adjust to your actual path
repo_config_path="$HOME/config-sync/app-config/config.json"

rm -f "$app_config_path"
ln -s "$repo_config_path" "$app_config_path"
```

With symlinks, whenever you edit settings in the app, they go straight into the repo checkout. You then `git commit` and `git push` those changes like normal.

### Option B: Sync scripts

If you prefer not to use symlinks, or they are inconvenient on a platform, use the scripts:

- `sync-app-config.ps1` on Windows
- `sync-app-config.sh` on Linux/macOS

Both scripts support two modes:

- **`pull`** – copy from repo → app config path
- **`push`** – copy from app config path → repo

You pass:

- the app config path (where SuperControlPanel stores `config.json`)
- the repo root path (where this `config-sync` repo is cloned)

Examples are in the script headers.

---

## 3. Git workflow

On a device where you update settings:

1. `git pull` to ensure you have latest config.
2. Run the sync script in **push** mode (or rely on symlink and just save settings in the app).
3. `git status` and verify `app-config/config.json` changed.
4. `git commit -am "Update SuperControlPanel config"` (or similar).
5. `git push`.

On another device before using the app:

1. `git pull`.
2. Run the sync script in **pull** mode (if not using symlink).
3. Launch SuperControlPanel – it now uses the shared config.

---

## 4. Optional: backups via Google Drive or a server

For extra durability:

- **Google Drive/Dropbox/OneDrive**  
  - Point your Git repo clone into a cloud-synced folder _or_ periodically export snapshots (e.g., zip archives of the repo) into that folder.
  - This provides an additional offsite backup and a fallback in case your Git host is temporarily unavailable.

- **Personal server/VPS**  
  - Keep a clone of this repo on your server.
  - Optionally set up a cron job/systemd timer that periodically runs `git fetch` and archives checkpoints (e.g., tarballs).

Git already gives you history and rollback; these options mainly protect against losing access to your Git host or local machines.

---

## 5. Secrets

SuperControlPanel stores sensitive data (like passwords) in the OS keyring, not in `config.json`.  
You should **keep that behavior** and avoid placing secrets directly in this repo.

If you later add other tools’ configs here, follow the same pattern:

- Keep secrets in OS keyring, password manager, or .env files that are **not** committed.
- Only sync non-sensitive configuration via Git.

