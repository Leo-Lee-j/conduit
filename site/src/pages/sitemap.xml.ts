import type { APIRoute } from 'astro';

export const GET: APIRoute = ({ site }) => {
  const base = `${import.meta.env.BASE_URL.replace(/\/$/, '')}/`;
  const pages = ['', 'zh-cn/'];
  return new Response(`<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${pages.map(path => `<url><loc>${new URL(`${base}${path}`, site)}</loc></url>`).join('')}</urlset>\n`, {
    headers: { 'Content-Type': 'application/xml; charset=utf-8' },
  });
};
