# Conduit website

The bilingual product site lives independently of the Rust app. English is at
`/conduit/`, Simplified Chinese at `/conduit/zh-cn/`. Both pages share the layout
in `src/components/Landing.astro` and copy in `src/content.ts`.

## Local development

Use Node 24 (at least the minimum specified in `package.json`):

```sh
cd site
npm ci
npm run dev
```

Open `http://127.0.0.1:4321/conduit/`. To validate the production output:

```sh
npm run check
npm run build
npm run preview
```

Check both languages at desktop and phone widths, language switching, screenshot
links, FAQ keyboard interaction, and download links. The site works without
client-side JavaScript. Build output and dependencies are ignored.

## GitHub Pages

1. In the repository's **Settings → Pages → Build and deployment**, set
   **Source** to **GitHub Actions**. This is a one-time repository setting;
   adding the workflow alone does not enable Pages.
2. Push the site to `main` or run **Website** manually on `main`.
3. The workflow checks and builds the site, uploads `site/dist`, and deploys to
   the `github-pages` environment. Pull requests only check and build.
4. Once deployed, use `https://leo-lee-j.github.io/conduit/` in the repository's
   About website field. Verify the English page, `/zh-cn/`, and direct assets
   on the live site.

The workflow uses the built-in GitHub token and needs no model credentials or
release signing secrets. It runs for site changes and changes to shared assets.
The existing Windows CI and release verification remain unchanged: a website
commit can also trigger Windows CI. Do not mark an unverified app commit as
verified merely because its website build passed.

`astro.config.mjs` owns the public origin and `/conduit` base path. If you later
use a custom domain, update both fields as appropriate and configure the domain
in Pages. Canonical links, language alternates, image URLs, and the sitemap use
these settings. `robots.txt` is emitted under the project path; crawler policy
for the whole `github.io` origin would need a root-level robots file.

## Content and assets

- Update both languages in `src/content.ts` together. Keep product claims aligned
  with the current README. The download button identifies its installer as
  Windows without making the operating system the headline.
- Downloads link to GitHub Releases' `/releases/latest`; do not hard-code an
  installer version or query GitHub's API in visitors' browsers.
- Detailed guides remain in `docs/`; link to them instead of maintaining a
  second copy. English links label Chinese-only guides.
- Import screenshots and logos from the existing repository assets. Astro emits
  responsive WebP images, with originals available through full-size links.
  Keep the screenshot demo note; the records are fictional.
- Engine marks retain their original SVG paths. The site publishes the upstream
  MIT notice at `third-party-notices.txt` and links to it from the footer.
- `public/social.png` is a 1200 × 630 sharing card composed from the existing
  app screenshot and logo. Regenerate with Pillow using
  `python site/scripts/generate-social.py` from the repository root. On another
  platform, pass `--font /path/to/font.ttf`. This optional tool is not needed
  for normal site builds.

Keep the Pages build separate from the mobile remote UI under `web/remote/`.
