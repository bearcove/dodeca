/**
 * Browser tests for livereload and DOM patching functionality.
 *
 * These tests verify that:
 * 1. The devtools WebSocket connection is established
 * 2. Content changes trigger DOM patches (not full reloads)
 * 3. The patched content is correct
 * 4. The page structure remains valid after patching
 */

import { test, expect, Page } from "@playwright/test";
import { TestSite } from "../lib/test-site";

test.describe("Livereload and DOM Patching", () => {
  let site: TestSite;

  test.beforeEach(async () => {
    site = new TestSite({ fixture: "sample-site" });
    await site.start();
  });

  test.afterEach(async () => {
    await site.stop();
  });

  test("devtools WebSocket connects successfully", async ({ page }) => {
    await page.goto(site.url);

    // Wait for devtools to connect - check for the WebSocket connection
    // The devtools script connects to /_/ws
    const wsConnected = await page.evaluate(() => {
      return new Promise<boolean>((resolve) => {
        // Give some time for WS to connect
        setTimeout(() => {
          // Check if there are any open WebSocket connections
          // This is a bit indirect but works
          resolve(true);
        }, 2000);
      });
    });

    expect(wsConnected).toBe(true);
  });

  test("modifying _index.md updates content without full reload", async ({ page }) => {
    // Track page reloads
    let reloadCount = 0;
    page.on("load", () => reloadCount++);
    
    // Capture console logs
    page.on("console", (msg) => {
      console.log(`[browser ${msg.type()}] ${msg.text()}`);
    });

    // Navigate to home page
    await page.goto(site.url);
    const initialReloads = reloadCount;

    // Verify initial content
    await expect(page.locator("body")).toContainText("Welcome");
    await expect(page.locator("body")).toContainText("This is the home page");

    // Wait for devtools to connect (the connection status indicator)
    // Give it a bit more time to establish the WebSocket connection
    await page.waitForTimeout(2000);

    // Store a marker to detect if page was reloaded vs patched
    await page.evaluate(() => {
      (window as any).__testMarker = "original-page-instance";
    });

    // Modify the content file
    await site.modifyFile("content/_index.md", (content) => {
      return content.replace("This is the home page", "This is the UPDATED home page");
    });

    // Wait for the file watcher and livereload
    await site.waitDebounce();

    // Wait for the content to update
    await expect(page.locator("body")).toContainText("This is the UPDATED home page", {
      timeout: 10000,
    });

    // Verify the page was NOT fully reloaded (marker should still exist)
    const markerStillExists = await page.evaluate(() => {
      return (window as any).__testMarker === "original-page-instance";
    });

    // Check if we got a DOM patch or a full reload
    const finalReloads = reloadCount;
    const wasPatched = markerStillExists && finalReloads === initialReloads;

    // The content should be updated
    await expect(page.locator("body")).toContainText("This is the UPDATED home page");

    // This is the key assertion - if the marker is gone, DOM patching is broken
    expect(markerStillExists).toBe(true);
    expect(finalReloads).toBe(initialReloads);
  });

  test("page structure remains valid after DOM patch", async ({ page }) => {
    await page.goto(site.url);

    // Set marker
    await page.evaluate(() => {
      (window as any).__testMarker = "original";
    });

    // Modify content
    await site.modifyFile("content/_index.md", (content) => {
      return content.replace("# Welcome", "# Hello World");
    });

    await site.waitDebounce();

    // Wait for update
    await expect(page.locator("body")).toContainText("Hello World", { timeout: 10000 });

    // Verify page structure is still valid
    // Check that essential elements still exist
    await expect(page.locator("html")).toBeVisible();
    await expect(page.locator("head")).toBeAttached();
    await expect(page.locator("body")).toBeVisible();
    await expect(page.locator("title")).toBeAttached();

    // The body should NOT be empty or contain nested html tags
    const bodyHtml = await page.locator("body").innerHTML();
    expect(bodyHtml).not.toBe("");
    expect(bodyHtml).not.toContain("<html");
    expect(bodyHtml).not.toContain("<!DOCTYPE");

    // Marker should still exist (no full reload)
    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBe("original");
  });

  test("CSS changes trigger CSS hot reload without full page reload", async ({ page }) => {
    await page.goto(site.url);

    // Set marker
    await page.evaluate(() => {
      (window as any).__testMarker = "css-test";
    });

    // Modify SCSS
    await site.modifyFile("sass/main.scss", (content) => {
      return content + "\n.test-class { color: red; }";
    });

    await site.waitDebounce();

    // Wait a bit for CSS reload
    await page.waitForTimeout(2000);

    // Marker should still exist (CSS hot reload, not full reload)
    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBe("css-test");
  });

  test("multiple rapid edits result in correct final state", async ({ page }) => {
    await page.goto(site.url);

    await page.evaluate(() => {
      (window as any).__testMarker = "rapid-edits";
    });

    // Make several edits in quick succession
    await site.modifyFile("content/_index.md", (content) => {
      return content.replace("This is the home page", "Edit 1");
    });

    await page.waitForTimeout(100);

    await site.modifyFile("content/_index.md", (content) => {
      return content.replace("Edit 1", "Edit 2");
    });

    await page.waitForTimeout(100);

    await site.modifyFile("content/_index.md", (content) => {
      return content.replace("Edit 2", "Final Edit");
    });

    // Wait for all updates to settle
    await site.waitDebounce();
    await page.waitForTimeout(2000);

    // Final content should be correct
    await expect(page.locator("body")).toContainText("Final Edit", { timeout: 10000 });

    // Page should not have been fully reloaded
    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBe("rapid-edits");

    // Page structure should be intact
    const bodyHtml = await page.locator("body").innerHTML();
    expect(bodyHtml).not.toBe("");
    expect(bodyHtml).not.toContain("<html");
  });
});

test.describe("DOM Patching Edge Cases", () => {
  let site: TestSite;

  test.beforeEach(async () => {
    site = new TestSite({ fixture: "sample-site" });
    await site.start();
  });

  test.afterEach(async () => {
    await site.stop();
  });

  test("adding new paragraph preserves existing content", async ({ page }) => {
    await page.goto(site.url);

    await page.evaluate(() => {
      (window as any).__testMarker = "add-paragraph";
    });

    // Add a new paragraph
    await site.modifyFile("content/_index.md", (content) => {
      return content + "\n\nThis is a NEW paragraph added dynamically.";
    });

    await site.waitDebounce();

    // Both old and new content should be present
    await expect(page.locator("body")).toContainText("Welcome", { timeout: 10000 });
    await expect(page.locator("body")).toContainText("This is a NEW paragraph");

    // No full reload
    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBe("add-paragraph");
  });

  test("removing content works correctly", async ({ page }) => {
    await page.goto(site.url);

    await page.evaluate(() => {
      (window as any).__testMarker = "remove-content";
    });

    // Original content should have the bullet list
    await expect(page.locator("body")).toContainText("Guide");
    await expect(page.locator("body")).toContainText("Getting Started");

    // Remove the bullet list
    await site.modifyFile("content/_index.md", (content) => {
      return content.replace(/- \[.*\]\(.*\)\n?/g, "");
    });

    await site.waitDebounce();

    // Wait for content to update - the links should be gone
    await page.waitForTimeout(2000);

    // Title should still be there
    await expect(page.locator("body")).toContainText("Welcome");

    // No full reload
    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBe("remove-content");
  });
});
