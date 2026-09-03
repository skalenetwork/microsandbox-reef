import type { APIRoute } from "astro";
import { description, posts, url } from "../content";

const escape = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const item = async (post: (typeof posts)[number]) => `    <item>
      <title>${escape(post.title)}</title>
      <link>${url(post.path)}</link>
      <guid isPermaLink="true">${url(post.path)}</guid>
      <pubDate>${new Date(post.date).toUTCString()}</pubDate>
      <description>${escape(post.summary)}</description>
      <content:encoded>${escape(await post.md.compiledContent())}</content:encoded>
    </item>`;

export const GET: APIRoute = async () =>
  new Response(
    `<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>reef</title>
    <link>${url("/blog")}</link>
    <description>${escape(description)}</description>
    <language>en</language>
    <atom:link href="${url("/rss.xml")}" rel="self" type="application/rss+xml" />
${(await Promise.all(posts.map(item))).join("\n")}
  </channel>
</rss>
`,
    { headers: { "Content-Type": "application/rss+xml; charset=utf-8" } },
  );
