---
name: web-code-style
description: Next.js + TypeScript code style for the quay web app.
paths:
  - "apps/web/**/*.ts"
  - "apps/web/**/*.tsx"
  - "apps/web/**/*.js"
  - "apps/web/**/*.jsx"
  - "apps/web/**/*.css"
  - "apps/web/**/package.json"
---

# Web Code Style — `apps/web/`

Next.js (App Router) + TypeScript + shadcn/ui + Tailwind. Biome for formatting + linting.

## 1. Formatting + linting

- `pnpm biome check --apply .` — formats + fixes safe lints.
- `pnpm biome ci .` runs in CI; failures block.
- No Prettier, no ESLint. Biome only.

## 2. TypeScript

- `"strict": true` in `tsconfig.json`. Non-negotiable.
- **No `any`**. Use `unknown` and narrow. `as` casts are a code smell — justify with a comment if used.
- Prefer `type` aliases for unions, `interface` for object shapes that may be extended.
- Public exports get an explicit return type.

## 3. App Router conventions

- **Server Components by default.** Add `"use client"` only when the component needs `useState`, `useEffect`, browser APIs, or event handlers.
- File conventions per [Next.js docs](https://nextjs.org/docs/app):
  - `app/<route>/page.tsx` — page
  - `app/<route>/layout.tsx` — shared layout
  - `app/<route>/loading.tsx` — Suspense fallback
  - `app/<route>/error.tsx` — error boundary (must be Client Component)
  - `app/<route>/route.ts` — Route Handler
  - `app/api/...` — API routes via Route Handlers
- **Server Actions** (`"use server"`) for mutations from forms. No standalone API routes for things a server action could do.
- Use `<Link>` for internal navigation. Never `<a href="/...">` for in-app routes.
- Use `<Image>` for images. Never raw `<img>` for app-served assets.

## 4. Data fetching

- `fetch()` in Server Components is the default. Pass `next: { revalidate: ... }` or `cache: 'no-store'` explicitly.
- Cache by default; opt out per request, not globally.
- Co-locate data fetching with the component that needs it. No separate `api/` client layer for data Server Components can fetch directly.
- Mutations: Server Actions + `revalidatePath` / `revalidateTag`.

## 5. Components

- shadcn/ui components live in `apps/web/components/ui/` (the shadcn convention). Don't edit them; wrap them.
- Domain components live in `apps/web/components/<domain>/`.
- One component per file. Filename = component name in PascalCase: `SkillCard.tsx`.
- Props typed with a `<Component>Props` type. No inline prop types in the function signature for non-trivial components.

## 6. Styling

- Tailwind utilities first. Use `cn()` (from `clsx` + `tailwind-merge`) to combine.
- No raw CSS unless you need a feature Tailwind can't express. Then put it in `globals.css` with a comment explaining why.
- Use CSS variables for theme tokens (`--background`, `--foreground`) — already shadcn's convention.
- Dark mode via `next-themes` + the `dark:` Tailwind variant.

## 7. State

- URL state for anything shareable: filters, pagination, selected tabs. Use `nuqs` or `useSearchParams`.
- Server state via Server Components + Server Actions. Don't reach for `react-query` unless you have a real client-state need.
- Client state via `useState` for local concerns. `Zustand` only when prop-drilling becomes painful.

## 8. Forms

- Server Action + `<form action={...}>`. Progressive enhancement out of the box.
- Validate on the server with `zod`. Validate on the client with the same schema for UX.
- Use [`react-hook-form`](https://react-hook-form.com/) with `@hookform/resolvers/zod` for complex forms; otherwise plain `<form>` with Server Action.

## 9. Imports

- Absolute imports via `@/` (configured in `tsconfig.json` paths).
- Group order: react/next, third-party, `@/lib`, `@/components`, `./local`. Biome enforces.
- No barrel files (`index.ts` re-exporting siblings). Slow builds, breaks tree-shaking.

## 10. Error handling

- Server-side errors throw. App Router's `error.tsx` boundary catches them.
- Server Actions return discriminated unions for expected failures: `{ ok: true, data } | { ok: false, error: string }`. Throw only for programmer errors.
- Never log secrets. Never include stack traces in user-facing error messages.

## 11. Forbidden

- `any`, `as any`, `@ts-ignore` (use `@ts-expect-error` with a comment if you must).
- `useEffect` for data fetching. Use Server Components or Server Actions.
- `localStorage` / `sessionStorage` for auth tokens. Use `httpOnly` cookies.
- Inline `<style>` blocks. Tailwind or `globals.css`.
- `dangerouslySetInnerHTML` without sanitization (DOMPurify or equivalent).
- Default exports for components. Named exports only — better refactor support.

## See also

- Companion skills: [`vercel-react-best-practices`](../skills/vercel-react-best-practices/), [`vercel-composition-patterns`](../skills/vercel-composition-patterns/), [`frontend-design`](../skills/frontend-design/)
- Test rules: [`web-testing.md`](web-testing.md)
- A11y: [`web-accessibility.md`](web-accessibility.md)
