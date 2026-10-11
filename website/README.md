# perritodeck.dev

Plain HTML and CSS, no build step. `index.html` is the landing page with a working demo deck; `privacy.html` is the privacy policy the Play Store listing links to.

## Preview

```sh
cd website && python3 -m http.server 8000   # then open http://localhost:8000
```

## Deploy

The repository is private, and GitHub Pages only serves private repos on paid plans, so the plan is **Cloudflare Pages** (free, works with private repos):

1. Cloudflare dashboard → Workers & Pages → Create → Pages → Connect to Git → pick `perritodeck/perritodeck`.
2. Framework preset: **None**. Build command: *(empty)*. Build output directory: `website`.
3. After the first deploy, Custom domains → add `perritodeck.dev` (and `www.perritodeck.dev`). If the domain's DNS is already on Cloudflare this is one click; otherwise point the nameservers at Cloudflare first.

Every push to `main` then redeploys the site automatically.

## Before launch

- Swap the "coming soon" rows in the download table for real links.
- The privacy policy already describes Phase 2 behaviour (pairing secret, host certificate); keep it in step with what the apps actually do.
