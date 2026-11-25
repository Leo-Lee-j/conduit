import type { APIRoute } from 'astro';
import notice from '../../../assets/icons/LICENSE.lobe-icons?raw';

export const GET: APIRoute = () => new Response(
  `Conduit engine icons\nSource: LobeHub Icons (@lobehub/icons-static-svg 1.98.0)\nhttps://github.com/lobehub/lobe-icons\nTrademarks belong to their respective owners.\n\n${notice}`,
  { headers: { 'Content-Type': 'text/plain; charset=utf-8' } },
);
