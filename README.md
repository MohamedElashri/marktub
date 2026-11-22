<p align="center">
  <img src="src-tauri/icons/256x256.png" width="72" height="72" alt="Marktub icon" />
</p>

<h1 align="center">Marktub</h1>

<p align="center"><strong>Read, write and review technical Markdown in your own files.</strong></p>
<p align="center">Offline · macOS and Linux · Free and open source</p>

<p align="center">
  <a href="https://github.com/MohamedElashri/marktub/releases/latest"><strong>Download the latest release</strong></a> ·
  <a href="#get-started">Installation</a> ·
  <a href="tests/fixtures/specimen.md">Feature guide</a> ·
  <a href="https://github.com/MohamedElashri/marktub/issues">Get help</a>
</p>

<p align="center">
  <a href="https://github.com/MohamedElashri/marktub/releases/latest"><img src="https://img.shields.io/github/v/release/MohamedElashri/marktub" alt="Latest release" /></a>
  <a href="https://github.com/MohamedElashri/marktub/actions/workflows/install.yml"><img src="https://github.com/MohamedElashri/marktub/actions/workflows/install.yml/badge.svg?branch=main" alt="Installer checks" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/MohamedElashri/marktub" alt="License" /></a>
</p>

## Why Marktub?

- **Ordinary files, kept local.** Open Markdown files and folders directly. Use
  tabs, search, bookmarks and linked notes, with no account, import or telemetry.
- **Technical writing, clearly rendered.** Read math, diagrams, tables and code,
  including Arabic and mixed RTL/LTR text. Write in Source or Visual, or use Split
  for a live preview beside your Markdown.
- **Review and share.** Inspect Git diffs, stage changes and leave revision
  feedback. Export standalone HTML or create PDFs with your selected local tools.

## Get started

Choose a package from the [latest release](https://github.com/MohamedElashri/marktub/releases/latest):

| Platform                            | Package          | Install                                          |
| ----------------------------------- | ---------------- | ------------------------------------------------ |
| macOS 13+ · Apple Silicon and Intel | Universal `.dmg` | Open the image and drag Marktub to Applications. |
| Linux x86_64 · Debian/Ubuntu        | `.deb`           | `sudo apt install ./Marktub_*_amd64.deb`         |
| Linux x86_64 · Fedora/RPM           | `.rpm`           | `sudo dnf install ./Marktub_*_x86_64.rpm`        |

Run Linux commands from the download folder. Linux packages require GTK 3,
WebKitGTK 4.1 and glibc 2.39 or newer. CI checks the installed app on Ubuntu 24.04
and RPM installation and dependencies on Fedora 44.

<details>
<summary>If macOS blocks the app</summary>

macOS packages are ad-hoc signed and unnotarized. Verify the official download
against the release’s `SHA256SUMS`, then follow Apple’s
[per-app opening instructions](https://support.apple.com/en-us/102445).
Alternatively, remove the installed app’s quarantine metadata:

```sh
xattr -dr com.apple.quarantine "/Applications/Marktub.app"
```

</details>

<details>
<summary>Install with curl or wget</summary>

The [installer](scripts/install.sh) detects your package format and verifies the
SHA-256 before opening the macOS image or installing through APT, DNF or Zypper:

```sh
curl -fsSL https://raw.githubusercontent.com/MohamedElashri/marktub/main/scripts/install.sh | bash
```

```sh
wget -qO- https://raw.githubusercontent.com/MohamedElashri/marktub/main/scripts/install.sh | bash
```

Append `-s -- --download-only` after `bash` to download without installing. Add
`--package-format deb` or `--package-format rpm` to choose a Linux format explicitly.

</details>

## Read and write

Open a file or folder, or start a **New note**. Choose **Read**, **Source**,
**Split** or **Visual**; **Format** opens writing tools and **More** holds secondary
document actions. Save your Markdown, or use **Export HTML** to share a copy.
**Focus reading** and writing focus give the document the whole workspace.

Open the built-in **Guide** for examples of formatting, linked notes, navigation,
themes, Git/Review, recovery and local tools. The same
[feature guide is available here](tests/fixtures/specimen.md).
PDF export requires Pandoc with Typst or XeLaTeX. Optional Shell, SQLite and TeX
jobs require explicit enablement, folder trust and confirmation.

## Updates

Use **Settings → About → Check for updates**. Checking, downloading and installing
are separate, explicit actions; downloads use Marktub’s pinned signing key.
On macOS, choose **Install and restart**. On Linux, approve the matching DEB or RPM
in the system installer, then choose **Restart after installation**.

## Contributing and license

See [development.md](development.md) for setup and checks. For bugs, include the OS,
app version and a small reproducible example in an
[issue](https://github.com/MohamedElashri/marktub/issues). Contributions are welcome.

Marktub is [MIT licensed](LICENSE).
