===== WAVE-52 RED RECEIPTS 2026-10-27T16:53:41+08:00 =====

### 1) UNIT (jsdom) — npx vitest run tests/ConfigGeneralSectionUpdateLayout.spec.ts
--- unit: npx vitest run tests/ConfigGeneralSectionUpdateLayout.spec.ts ---
       |                                                                                ^
    129|     expect(wrapper.find('[data-testid="config-update-last-checked"]').…
    130|   });

⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯[4/4]⎯


 Test Files  1 failed (1)
      Tests  4 failed (4)
   Start at  16:52:24
   Duration  2.36s (transform 769ms, setup 0ms, import 1.59s, tests 171ms, environment 435ms)

unit_exit=1

### 2) E2E (真实 Chromium) — npx playwright test tests/e2e/wave52-update-card-layout.spec.ts --retries=0 --grep 'C1|C2|C3'
  ✘  1 [chromium] › tests/e2e/wave52-update-card-layout.spec.ts:50:1 › C1：分区标题为「软件自动更新」（真实浏览器文本） (21.3s)
  ✘  2 [chromium] › tests/e2e/wave52-update-card-layout.spec.ts:61:1 › C2：Switch 是该行最右端元素（物理几何 ≤2px 贴边，文案在左） (21.0s)
  ✘  3 [chromium] › tests/e2e/wave52-update-card-layout.spec.ts:104:1 › C3：「上次检查」与当前版本同一行、且位于版本 chip 右侧（≤4px 同排） (21.0s)
    Error: [2mexpect([22m[31mlocator[39m[2m).[22mtoBeVisible[2m([22m[2m)[22m failed
    Locator: getByTestId('config-update-section-title')
    Expected: visible
    Error: element(s) not found
    Error: [2mexpect([22m[31mlocator[39m[2m).[22mtoBeVisible[2m([22m[2m)[22m failed
    Locator: getByTestId('config-update-autocheck-row')
    Expected: visible
    Error: element(s) not found
    Error: [2mexpect([22m[31mlocator[39m[2m).[22mtoBeVisible[2m([22m[2m)[22m failed
    Locator: getByTestId('config-update-version-row')
    Expected: visible
    Error: element(s) not found
  3 failed
e2e_exit=1
