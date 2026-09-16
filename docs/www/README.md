# Spec Chum public site (`docs/www/`)

Self-contained static marketing landing for **GitHub Pages** (and optional
custom hosting). Structure follows a simple product home — brand, short pitch,
download CTA, platforms/features — not a docs SPA.

| Path | Role |
| --- | --- |
| [`index.html`](index.html) | Marketing page (canonical) |
| [`styles.css`](styles.css) | OKLCH palette + layout |
| [`site.js`](site.js) | Light motion helpers |
| [`assets/`](assets/) | App icon PNGs |
| [`fonts/`](fonts/) | Self-hosted woff2 (SIL OFL) |
| [`.nojekyll`](.nojekyll) | Disable Jekyll processing if served from a branch |

Markdown under `docs/*.md` remains the player/developer docs index on GitHub.
Do not put LLM agent instructions on this marketing page.

Asset URLs are **relative** (`./styles.css`, `./assets/…`, `./fonts/…`) so the
folder can move to a path prefix or another host without rewriting CSS/JS/image
links. Fonts are **self-hosted** woff2 under `fonts/` (Space Grotesk, IBM Plex
Sans — SIL OFL); no Google Fonts / third-party font CDN at runtime.

## Palette

See the header comment in `styles.css`. Tokens use **OKLCH**, inspired by the ZX
Spectrum ULA attribute set (`ula::palette_rgb`) — hardware colour language only.
No Sinclair/Amstrad logos or trademarked rainbow wordmark.

## When the live site updates

The public site is **not** redeployed on every push to `main`.

GitHub Actions workflow [`.github/workflows/pages.yml`](../../.github/workflows/pages.yml)
deploys the **contents** of this folder as the Pages **site root** when:

1. A **GitHub Release is published** (normal path: push a `vX.Y.Z` tag →
   [`.github/workflows/release.yml`](../../.github/workflows/release.yml)
   packages apps and publishes the release → Pages workflow runs on
   `release: published`), or
2. Someone runs **Actions → GitHub Pages → Run workflow**
   (`workflow_dispatch`) for a manual smoke deploy.

Live URL shape:
`https://<owner>.github.io/spec_chum/` (marketing `index.html` at the root).

See also [docs/RELEASE.md](../RELEASE.md) (release checklist includes the site).

## Enable GitHub Pages (GitHub Actions)

One-time repo setup (maintainers):

1. Repo **Settings → Pages**.
2. **Source:** **GitHub Actions** (not “Deploy from a branch”).
3. Save. First content appears after the next successful Pages workflow run
   (publish a release, or `workflow_dispatch`).

Optional CLI (needs admin / Pages write):

```bash
gh api repos/<owner>/spec_chum/pages -X PUT -f build_type=workflow
```

If the `github-pages` environment restricts deployment branches, allow the
refs you deploy from (at least `main`, and tags matching `v*` for release
publishes), or disable custom branch policies for that environment.

## Custom domain later (scripthungry.com — document only)

Do **not** commit a live `CNAME` for `scripthungry.com` until DNS is ready.
Two common options:

### Subdomain (e.g. `specchum.scripthungry.com`)

1. In DNS for `scripthungry.com`, add a **CNAME** record:
   `specchum` → `<owner>.github.io`
2. In GitHub **Settings → Pages → Custom domain**, enter
   `specchum.scripthungry.com` and wait for DNS check / HTTPS.
3. GitHub may add a `CNAME` file under the Pages publish root — review that
   change when you enable it; still keep this `www/` tree portable.

The site then loads at the subdomain root (same artifact: contents of
`docs/www/`).

### Folder / path (e.g. `scripthungry.com/spec-chum/`)

1. Publish or sync the **contents** of `docs/www/` to that path on the main site
   (static host, Pages on another repo, or reverse proxy).
2. Relative assets keep working under `/spec-chum/` because links are `./…`.
3. Point any marketing URLs at `https://scripthungry.com/spec-chum/` (trailing
   slash preferred).

Subdomain is usually simpler with GitHub Pages custom domains; folder hosting
fits when Spec Chum is one section of an existing site.

## Local preview

```bash
# from repo root
python3 -m http.server 8765 --directory docs/www
# http://127.0.0.1:8765/
```
