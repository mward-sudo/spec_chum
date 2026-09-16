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
| [`../index.html`](../index.html) | Redirect → `./www/` when Pages serves `/docs` |

Markdown under `docs/*.md` remains the player/developer docs index on GitHub.
Do not put LLM agent instructions on this marketing page.

Asset URLs are **relative** (`./styles.css`, `./assets/…`) so the folder can move
to a path prefix or another host without rewriting CSS/JS/image links.

## Palette

See the header comment in `styles.css`. Tokens use **OKLCH**, inspired by the ZX
Spectrum ULA attribute set (`ula::palette_rgb`) — hardware colour language only.
No Sinclair/Amstrad logos or trademarked rainbow wordmark.

## Enable GitHub Pages

1. Repo **Settings → Pages**.
2. **Source:** Deploy from a branch.
3. **Branch:** `main`, folder **`/docs`**.
4. Save. Default URL shape:
   `https://<owner>.github.io/spec_chum/` → redirects to `/www/`.
   Canonical page: `https://<owner>.github.io/spec_chum/www/`.

`docs/.nojekyll` disables Jekyll so files are served as-is.

Optional CLI (needs Pages write permission):

```bash
gh api repos/<owner>/spec_chum/pages -X POST \
  -f build_type=legacy \
  -f source[branch]=main \
  -f source[path]=/docs
```

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

The site then loads at the subdomain root (serve/copy `docs/www/` contents, or
keep `/docs` publish and use the `/www/` path / redirect as today).

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
# from repo root — open the portable folder directly
python3 -m http.server 8765 --directory docs/www
# http://127.0.0.1:8765/

# or full /docs tree (redirect + www)
python3 -m http.server 8765 --directory docs
# http://127.0.0.1:8765/www/
```
