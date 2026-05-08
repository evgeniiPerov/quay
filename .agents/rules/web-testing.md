---
name: web-testing
description: Testing rules for the Next.js web app — Vitest for units, Playwright for e2e, real DB via testcontainers.
paths:
  - "apps/web/**/*.ts"
  - "apps/web/**/*.tsx"
  - "apps/web/**/test/**"
  - "apps/web/**/__tests__/**"
  - "apps/web/**/*.test.ts"
  - "apps/web/**/*.test.tsx"
  - "apps/web/**/*.spec.ts"
  - "apps/web/**/*.spec.tsx"
---

# Web Testing — `apps/web/`

## 1. Test type matrix

| What you're testing                       | Test type      | Tooling                                  |
|--------------------------------------------|----------------|------------------------------------------|
| Pure utility (`lib/`, `utils/`)            | unit           | Vitest                                   |
| Server Component output                    | integration    | Vitest + `@testing-library/react`        |
| Client Component interactivity             | component      | Vitest + `@testing-library/react` + `user-event` |
| Server Action / Route Handler              | integration    | Vitest + real DB (testcontainers)        |
| Full-page user flow                        | e2e            | Playwright against running dev server    |
| Visual regression                          | screenshot     | Playwright `expect(page).toHaveScreenshot()` |
| Accessibility (axe rules)                  | e2e            | Playwright + `@axe-core/playwright`      |

## 2. Unit + component tests (Vitest)

- Co-located: `MyComponent.tsx` + `MyComponent.test.tsx` in the same directory.
- Use `@testing-library/react` queries: `getByRole`, `getByLabelText`. Avoid `getByTestId` unless you have no choice.
- Test behavior, not implementation. "User sees X after clicking Y", not "internal state is Z".
- `user-event` over `fireEvent`. It simulates real interaction.

## 3. Server Action / Route Handler tests

- Spin up a real Postgres via [`@testcontainers/postgresql`](https://node.testcontainers.org/modules/postgresql/) per test file.
- Run migrations against the container before tests. Tear down after.
- **Do not mock the DB.** Mocking ORM calls means you're testing your mock, not your code.
- Mock external HTTP only at the `fetch` boundary using [MSW](https://mswjs.io/). Mock the network, not your code.

## 4. End-to-end tests (Playwright)

```
apps/web/e2e/
├── auth.spec.ts
├── skills.spec.ts
└── fixtures/
    └── test-data.ts
```

- One spec file per user-facing flow.
- `playwright.config.ts` runs against the dev server (`pnpm dev`) or a built+started production server (`pnpm build && pnpm start`) — pick one and stick with it.
- Use the [`anthropics/skills@webapp-testing`](../skills/webapp-testing/) skill for Playwright authoring guidance.
- **Real browser, real network for happy paths.** Mock only when testing the unmockable (e.g. third-party OAuth in CI).
- Test data: each spec creates its own data and cleans up. No shared global fixtures.

## 5. Visual regression

- `await expect(page).toHaveScreenshot('skill-card.png')` for components with stable visual contracts.
- Snapshots committed under `apps/web/e2e/__screenshots__/`.
- Review snapshot changes manually: `pnpm playwright test --update-snapshots` only after eyeballing.
- Deterministic rendering: stub `Date.now()`, fonts, animations. Use Playwright's `--reduced-motion` and freeze the clock.

## 6. Accessibility tests

- Every page-level e2e spec runs an axe check:
  ```ts
  import { AxeBuilder } from '@axe-core/playwright';
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  ```
- Component-level a11y checks live in unit tests with `jest-axe` (`vitest-axe` if available).

## 7. Coverage

- Vitest coverage via `--coverage` (uses v8). Target: 80% lines on `lib/`, lower bar on UI components.
- Coverage is a smell detector, not a goal. 100% coverage on a useless test is worse than 60% on real ones.
- CI fails if coverage drops more than 2pp from `main`.

## 8. CI

```
pnpm biome ci .
pnpm tsc --noEmit
pnpm vitest run --coverage
pnpm playwright test
```

Playwright runs on a matrix: chromium, firefox, webkit. Mobile viewport (iPhone 14) included.

## 9. Forbidden

- Mocking the database. Use testcontainers.
- `screen.debug()` left in committed code.
- `it.skip` or `test.skip` without a linked issue and a deadline.
- Tests that depend on `setTimeout` for synchronization. Use `await waitFor(...)` or Playwright's auto-waiting.
- Tests that rely on production data, production URLs, or shared accounts.
- Snapshot tests of large JSON blobs that nobody will review meaningfully — those are commitment to noise, not a real assertion.

## See also

- Companion skill: [`anthropics/skills@webapp-testing`](../skills/webapp-testing/)
- Style: [`web-code-style.md`](web-code-style.md)
- A11y: [`web-accessibility.md`](web-accessibility.md)
