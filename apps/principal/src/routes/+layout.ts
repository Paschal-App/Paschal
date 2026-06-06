// SPA mode: every route is client-rendered. SvelteKit's adapter-static
// reads this and produces the single index.html fallback.

export const ssr = false;
export const prerender = false;
export const trailingSlash = 'never';
