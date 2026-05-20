import { defineCollection, z } from "astro:content";
import { docsSchema } from "@astrojs/starlight/schema";

const docs = defineCollection({
  schema: docsSchema(),
});

const blog = defineCollection({
  type: "content",
  schema: z.object({
    title: z.string(),
    description: z.string(),
    date: z.date(),
    author: z.string(),
    authorUrl: z.string().url().optional(),
    tags: z.array(z.string()).optional(),
    draft: z.boolean().default(false),
    image: z.string().optional(),
  }),
});

export const collections = { docs, blog };
