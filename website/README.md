# pgvis Website

The [pgvis.io](https://pgvis.io) website — landing page, documentation, guides, and blog.

Built with [Astro](https://astro.build/) and [Starlight](https://starlight.astro.build/).

## Development

```bash
cd website

# Install dependencies
npm install

# Start dev server
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview
```

## Structure

```
website/
├── astro.config.mjs           # Astro + Starlight configuration
├── src/
│   ├── content.config.ts      # Content collection schemas
│   ├── content/
│   │   ├── docs/              # Documentation (Starlight)
│   │   │   ├── introduction.mdx
│   │   │   ├── quickstart.mdx
│   │   │   ├── installation.mdx
│   │   │   ├── guides/        # How-to guides
│   │   │   └── reference/     # API/config reference
│   │   └── blog/              # Blog posts
│   ├── pages/                 # Custom pages (landing, blog index)
│   ├── layouts/               # Page layouts
│   ├── components/            # Reusable UI components
│   ├── styles/                # Global styles
│   └── assets/                # Logos, images
└── public/                    # Static files served as-is
```

## Adding Content

### New Documentation Page

Create a `.mdx` file in `src/content/docs/`:

```mdx
---
title: Your Page Title
description: A brief description.
---

# Your Content Here
```

Then add it to the sidebar in `astro.config.mjs`.

### New Blog Post

Create a `.mdx` file in `src/content/blog/`:

```mdx
---
title: "Your Post Title"
description: "A brief description."
date: 2025-01-15
author: "Your Name"
tags: ["tag1", "tag2"]
---

# Your Blog Post
```

### New Guide

Create a `.mdx` file in `src/content/docs/guides/`. Guides are auto-generated in the sidebar via the `autogenerate` config.

## Deployment

The website is automatically deployed via GitHub Actions on push to `main` when files in `website/` change. See `.github/workflows/website.yml`.

Currently configured for GitHub Pages. To switch to Vercel/Cloudflare Pages/Netlify, update the workflow accordingly.
