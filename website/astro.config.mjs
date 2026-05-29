import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import mdx from "@astrojs/mdx";
import sitemap from "@astrojs/sitemap";

const SITE = "https://pgvis.io";

// https://astro.build/config
export default defineConfig({
  site: SITE,
  integrations: [
    starlight({
      title: "pgvis",
      description:
        "Turn any Postgres database into MCP tools, a PostgREST-compatible REST API, and an OpenAPI 3.0 spec — from one I/O-free Rust engine.",
      logo: {
        src: "./src/assets/app-icon-indigo.svg",
        replacesTitle: false,
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/pgvis/pgvis",
        },
      ],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Introduction", slug: "introduction" },
            { label: "Quick Start", slug: "quickstart" },
            { label: "Installation", slug: "installation" },
          ],
        },
        {
          label: "Guides",
          autogenerate: { directory: "guides" },
        },
        {
          label: "Reference",
          autogenerate: { directory: "reference" },
        },
      ],
      customCss: ["./src/styles/custom.css"],
      components: {
        Footer: "./src/components/Footer.astro",
      },
      head: [
        {
          tag: "meta",
          attrs: { name: "theme-color", content: "#6366f1" },
        },
        {
          tag: "meta",
          attrs: {
            name: "keywords",
            content:
              "pgvis, PostgREST alternative, MCP server, Model Context Protocol, Postgres REST API, OpenAPI 3.0, Rust database API, embeddable database API, LLM database tools",
          },
        },
        {
          tag: "link",
          attrs: { rel: "icon", type: "image/svg+xml", href: "/app-icon-indigo.svg" },
        },
      ],
    }),
    mdx(),
    sitemap({
      changefreq: "weekly",
      priority: 0.7,
    }),
  ],
});
