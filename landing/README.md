# Neptunus landing page

Static page (HTML + Tailwind CSS via CDN). No build step.

## Deploy to Cloudflare Pages

Dashboard: Workers & Pages › Create › Pages › Connect to Git, then:

- Framework preset: **None**
- Build command: *(empty)*
- Build output directory: `landing`

Or from the CLI, run this from the repository root:

```bash
npx wrangler pages deploy landing --project-name neptunus
```

## Local preview

```bash
cd landing && python3 -m http.server 8080
```
