import { test, expect } from "@playwright/test";

test.describe("newline debugging", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/index.html");
    await page.waitForFunction(() => (window as any).wasmReady === true, { timeout: 10000 });
  });

  test("h1 removed - debug patches", async ({ page }) => {
    const logs: string[] = [];
    page.on("console", msg => logs.push(msg.text()));
    
    const result = await page.evaluate(() => {
      const oldHtml = '<my-header>\n      <h1 slot="title">App Title</h1>\n    </my-header>';
      const newHtml = '<my-header>\n      中文App Title\n    </my-header>';
      
      const fullOld = "<html><body>" + oldHtml + "</body></html>";
      const fullNew = "<html><body>" + newHtml + "</body></html>";
      
      const patchesJson = (window as any).diffHtml(fullOld, fullNew);
      console.log("PATCHES: " + patchesJson);
      
      // Parse patches to inspect text values
      const patches = JSON.parse(patchesJson);
      for (let i = 0; i < patches.length; i++) {
        const p = patches[i];
        console.log("PATCH " + i + ": " + JSON.stringify(p));
        
        // Check for text values
        if (p.SetText) {
          console.log("  SetText.text = " + JSON.stringify(p.SetText.text));
          console.log("  Has actual newline: " + p.SetText.text.includes("\n"));
          console.log("  Char codes: " + Array.from(p.SetText.text).map((c: string) => c.charCodeAt(0)).join(","));
        }
        if (p.InsertText) {
          console.log("  InsertText.text = " + JSON.stringify(p.InsertText.text));
        }
      }
      
      (window as any).setBodyInnerHtml(oldHtml);
      (window as any).applyPatchesJson(patchesJson);
      const resultHtml = (window as any).getBodyInnerHtml();
      
      console.log("RESULT: " + JSON.stringify(resultHtml));
      console.log("EXPECTED: " + JSON.stringify(newHtml));
      
      const normalize = (html: string) => {
        const temp = document.createElement("div");
        temp.innerHTML = html;
        return temp.innerHTML;
      };
      
      return {
        pass: normalize(resultHtml) === normalize(newHtml),
        result: resultHtml,
        expected: newHtml,
        patches: patchesJson,
      };
    });

    console.log("=== Console logs ===");
    for (const log of logs) {
      console.log(log);
    }
    
    expect(result.pass, "Got: " + result.result + "\nExpected: " + result.expected).toBe(true);
  });
});
