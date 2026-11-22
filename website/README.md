# shellmate website

The documentation site, built with [Astro Starlight](https://starlight.astro.build/) as a fully static site.

Needs Node.js 22.23.4 or newer (`.nvmrc` pins 26) and [pnpm](https://pnpm.io/) (the version is pinned in `package.json`; `corepack enable` picks it up).

```sh
cd website
pnpm install
pnpm dev           # http://localhost:4321/  (landing page)  and  /docs/  (documentation)
pnpm build         # static output in dist/
pnpm preview       # serve dist/
```

## Layout

- `/` is the landing page: `src/pages/index.astro` (plain Astro, no Starlight).
- `/docs/...` is the documentation. The Markdown/MDX files live in `src/content/docs/docs/`, so their URLs get the `/docs` prefix; the sidebar is defined in `astro.config.mjs` (slugs start with `docs/`).
- Links between documentation pages are relative (`../privacy/`). Links built in code use `import.meta.env.BASE_URL`, so nothing hard-codes a prefix.

## Deployment

Locally, the site is built for the root of a domain.

```sh
SITE=https://example.org pnpm build
```

`SITE` is the public URL; without it canonical links and the sitemap are skipped. To serve from a sub-path instead (for example GitHub Pages without a custom domain) also set `BASE=/shellmate`.

`.github/workflows/website.yml` builds and deploys to GitHub Pages when `website/` changes on `main`. In the repository settings, set Pages to "GitHub Actions". By default the workflow builds for `https://paulomenin.github.io/shellmate` (`SITE=https://paulomenin.github.io`, `BASE=/shellmate`). To serve from a custom domain, add the domain under Pages and create the repository variables `SITE` with the public URL and `BASE` with `/`.
