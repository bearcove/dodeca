/**
 * Standalone DOM patching tests.
 * 
 * These tests verify that DOM patching works correctly in a real browser,
 * independent of dodeca's WebSocket infrastructure.
 */

import { test, expect } from "@playwright/test";

// Inject the patching functions into the page
async function injectPatchFunctions(page: any) {
    await page.addScriptTag({
        content: `
            window.findNode = function(path) {
                let current = document.body;
                for (const idx of path) {
                    const children = current.childNodes;
                    if (idx >= children.length) {
                        throw new Error('Child index ' + idx + ' out of bounds (' + children.length + ' children)');
                    }
                    current = children[idx];
                }
                return current;
            };

            window.findElement = function(path) {
                const node = window.findNode(path);
                if (node.nodeType !== 1) {
                    throw new Error('Node at path [' + path.join(', ') + '] is not an element');
                }
                return node;
            };

            window.extractBodyInner = function(html) {
                const match = html.match(/<body[^>]*>([\\s\\S]*)<\\/body>/i);
                return match ? match[1] : null;
            };

            window.applyPatch = function(patch) {
                console.log('Applying patch:', JSON.stringify(patch));
                
                switch (patch.type) {
                    case 'SetText': {
                        const node = window.findNode(patch.path);
                        node.textContent = patch.text;
                        break;
                    }
                    case 'SetAttribute': {
                        const el = window.findElement(patch.path);
                        el.setAttribute(patch.name, patch.value);
                        break;
                    }
                    case 'RemoveAttribute': {
                        const el = window.findElement(patch.path);
                        el.removeAttribute(patch.name);
                        break;
                    }
                    case 'Remove': {
                        const node = window.findNode(patch.path);
                        node.parentNode.removeChild(node);
                        break;
                    }
                    case 'Replace': {
                        const el = window.findElement(patch.path);
                        if (patch.path.length === 0) {
                            const inner = window.extractBodyInner(patch.html);
                            if (inner !== null) {
                                el.innerHTML = inner;
                            } else {
                                el.outerHTML = patch.html;
                            }
                        } else {
                            el.outerHTML = patch.html;
                        }
                        break;
                    }
                    case 'InsertBefore': {
                        const el = window.findElement(patch.path);
                        el.insertAdjacentHTML('beforebegin', patch.html);
                        break;
                    }
                    case 'InsertAfter': {
                        const el = window.findElement(patch.path);
                        el.insertAdjacentHTML('afterend', patch.html);
                        break;
                    }
                    case 'AppendChild': {
                        const el = window.findElement(patch.path);
                        el.insertAdjacentHTML('beforeend', patch.html);
                        break;
                    }
                    default:
                        throw new Error('Unknown patch type: ' + patch.type);
                }
            };

            window.applyPatches = function(patches) {
                for (const patch of patches) {
                    window.applyPatch(patch);
                }
                return patches.length;
            };
        `
    });
}

test.describe("Standalone DOM Patching", () => {
    test("Replace at empty path updates body content", async ({ page }) => {
        await page.setContent(`
            <!DOCTYPE html>
            <html>
            <head><title>Test</title></head>
            <body>
                <h1>Welcome</h1>
                <p>This is the home page.</p>
            </body>
            </html>
        `);

        await injectPatchFunctions(page);
        await expect(page.locator("body")).toContainText("This is the home page");

        await page.evaluate(() => {
            (window as any).applyPatches([{
                type: 'Replace',
                path: [],
                html: '<body><h1>Welcome</h1><p>This is the UPDATED home page.</p></body>'
            }]);
        });

        await expect(page.locator("body")).toContainText("This is the UPDATED home page");
    });

    test("Replace at empty path with complex HTML", async ({ page }) => {
        await page.setContent(`
            <!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="utf-8">
                <title>Test Page</title>
            </head>
            <body>
                <nav>
                    <h1>Home</h1>
                </nav>
                <main>
                    <h1>Welcome</h1>
                    <p>This is the home page.</p>
                    <ul>
                        <li><a href="/guide/">Guide</a></li>
                    </ul>
                </main>
            </body>
            </html>
        `);

        await injectPatchFunctions(page);
        await expect(page.locator("body")).toContainText("This is the home page");

        await page.evaluate(() => {
            (window as any).applyPatches([{
                type: 'Replace',
                path: [],
                html: `<body>
                    <nav>
                        <h1>Home</h1>
                    </nav>
                    <main>
                        <h1>Welcome</h1>
                        <p>This is the UPDATED home page.</p>
                        <ul>
                            <li><a href="/guide/">Guide</a></li>
                        </ul>
                    </main>
                </body>`
            }]);
        });

        await expect(page.locator("body")).toContainText("This is the UPDATED home page");
        await expect(page.locator("body")).toContainText("Home");
        await expect(page.locator("body")).toContainText("Guide");
    });

    test("SetText updates text content", async ({ page }) => {
        // No whitespace between body and p to avoid text nodes
        await page.setContent(`<!DOCTYPE html><html><body><p>Original text</p></body></html>`);

        await injectPatchFunctions(page);
        await expect(page.locator("p")).toContainText("Original text");

        await page.evaluate(() => {
            (window as any).applyPatches([{
                type: 'SetText',
                path: [0],  // First child of body (the <p>)
                text: 'Updated text'
            }]);
        });

        await expect(page.locator("p")).toContainText("Updated text");
    });

    test("SetAttribute updates attribute", async ({ page }) => {
        // No whitespace between body and a to avoid text nodes
        await page.setContent(`<!DOCTYPE html><html><body><a href="/old-link">Click me</a></body></html>`);

        await injectPatchFunctions(page);
        expect(await page.locator("a").getAttribute("href")).toBe("/old-link");

        await page.evaluate(() => {
            (window as any).applyPatches([{
                type: 'SetAttribute',
                path: [0],
                name: 'href',
                value: '/new-link'
            }]);
        });

        expect(await page.locator("a").getAttribute("href")).toBe("/new-link");
    });

    test("Remove deletes element", async ({ page }) => {
        // No whitespace between elements to avoid text nodes
        await page.setContent(`<!DOCTYPE html><html><body><p>Keep me</p><p>Delete me</p></body></html>`);

        await injectPatchFunctions(page);
        expect(await page.locator("p").count()).toBe(2);

        await page.evaluate(() => {
            (window as any).applyPatches([{
                type: 'Remove',
                path: [1]  // Second child (second <p>)
            }]);
        });

        expect(await page.locator("p").count()).toBe(1);
        await expect(page.locator("p")).toContainText("Keep me");
    });
});
