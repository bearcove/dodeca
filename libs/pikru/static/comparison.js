/**
 * Pikchr Comparison Tool - Interactive SVG Comparison
 *
 * Features:
 * - Side-by-side view (default)
 * - Difference view (pixel diff)
 * - Onion skin overlay with opacity slider
 * - Swipe/drag comparison
 * - On-demand SSIM calculation
 */

(function() {
  'use strict';

  // View modes
  const MODES = {
    SIDE_BY_SIDE: 'side-by-side',
    DIFFERENCE: 'difference',
    ONION_SKIN: 'onion-skin',
    SWIPE: 'swipe'
  };

  // State per card
  const cardStates = new Map();

  // Initialize when DOM is ready
  document.addEventListener('DOMContentLoaded', init);

  function init() {
    // Find all test cards and enhance them
    document.querySelectorAll('.test-card').forEach(card => {
      enhanceCard(card);
    });

    // Add global keyboard shortcuts
    document.addEventListener('keydown', handleGlobalKeydown);
  }

  function enhanceCard(card) {
    const cardId = card.id;
    const comparison = card.querySelector('.comparison');
    if (!comparison) return;

    const cColumn = comparison.querySelector('.c-output');
    const rustColumn = comparison.querySelector('.rust-output');
    if (!cColumn || !rustColumn) return;

    const cSvgContainer = cColumn.querySelector('.svg-container');
    const rustSvgContainer = rustColumn.querySelector('.svg-container');
    if (!cSvgContainer || !rustSvgContainer) return;

    // Check if both have actual SVGs (not errors)
    const cSvg = cSvgContainer.querySelector('svg');
    const rustSvg = rustSvgContainer.querySelector('svg');

    // Initialize state
    cardStates.set(cardId, {
      mode: MODES.SIDE_BY_SIDE,
      opacity: 0.5,
      swipePosition: 50,
      ssim: null,
      cSvg,
      rustSvg
    });

    // Only add controls if both sides have SVGs
    if (cSvg && rustSvg) {
      addModeControls(card, comparison);
    }
  }

  function addModeControls(card, comparison) {
    const cardId = card.id;
    const state = cardStates.get(cardId);

    // Create controls container
    const controls = document.createElement('div');
    controls.className = 'compare-controls';
    controls.innerHTML = `
      <div class="mode-buttons">
        <button class="mode-btn active" data-mode="${MODES.SIDE_BY_SIDE}" title="Side by Side (1)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="1" y="2" width="6" height="12" rx="1" stroke="currentColor" stroke-width="1.5"/>
            <rect x="9" y="2" width="6" height="12" rx="1" stroke="currentColor" stroke-width="1.5"/>
          </svg>
        </button>
        <button class="mode-btn" data-mode="${MODES.DIFFERENCE}" title="Difference View (2)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <circle cx="6" cy="8" r="5" stroke="currentColor" stroke-width="1.5"/>
            <circle cx="10" cy="8" r="5" stroke="currentColor" stroke-width="1.5"/>
          </svg>
        </button>
        <button class="mode-btn" data-mode="${MODES.ONION_SKIN}" title="Onion Skin (3)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="2" y="3" width="10" height="10" rx="1" stroke="currentColor" stroke-width="1.5" opacity="0.4"/>
            <rect x="4" y="5" width="10" height="10" rx="1" stroke="currentColor" stroke-width="1.5"/>
          </svg>
        </button>
        <button class="mode-btn" data-mode="${MODES.SWIPE}" title="Swipe Compare (4)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="1" y="2" width="14" height="12" rx="1" stroke="currentColor" stroke-width="1.5"/>
            <line x1="8" y1="2" x2="8" y2="14" stroke="currentColor" stroke-width="1.5"/>
            <path d="M5 8L3 6M5 8L3 10M11 8L13 6M11 8L13 10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          </svg>
        </button>
      </div>
      <div class="mode-slider" style="display: none;">
        <label>Opacity</label>
        <input type="range" min="0" max="100" value="50" class="opacity-slider">
        <span class="slider-value">50%</span>
      </div>
      <button class="ssim-btn" title="Calculate SSIM similarity score">
        <span class="ssim-label">SSIM</span>
        <span class="ssim-value">—</span>
      </button>
    `;

    // Insert controls before the comparison grid
    comparison.parentNode.insertBefore(controls, comparison);

    // Wire up mode buttons
    controls.querySelectorAll('.mode-btn').forEach(btn => {
      btn.addEventListener('click', () => {
        setMode(card, btn.dataset.mode);
      });
    });

    // Wire up opacity slider
    const slider = controls.querySelector('.opacity-slider');
    const sliderValue = controls.querySelector('.slider-value');
    slider.addEventListener('input', (e) => {
      state.opacity = e.target.value / 100;
      sliderValue.textContent = `${e.target.value}%`;
      // Update the overlay directly without full re-render for smoother feedback
      const overlay = card.querySelector('.overlay-rust');
      if (overlay) {
        overlay.style.opacity = state.opacity;
        const rustLabel = card.querySelector('.overlay-labels span:last-child');
        if (rustLabel) {
          rustLabel.textContent = `Rust (${e.target.value}%)`;
        }
      }
    });

    // Wire up SSIM button
    controls.querySelector('.ssim-btn').addEventListener('click', () => {
      calculateSSIM(card);
    });
  }

  function setMode(card, mode) {
    const cardId = card.id;
    const state = cardStates.get(cardId);
    state.mode = mode;

    // Update button states
    const controls = card.querySelector('.compare-controls');
    controls.querySelectorAll('.mode-btn').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.mode === mode);
    });

    // Show/hide slider for onion skin mode
    const sliderContainer = controls.querySelector('.mode-slider');
    sliderContainer.style.display = mode === MODES.ONION_SKIN ? 'flex' : 'none';

    updateView(card);
  }

  function updateView(card) {
    const cardId = card.id;
    const state = cardStates.get(cardId);
    const comparison = card.querySelector('.comparison');

    // Remove any existing overlay containers
    comparison.querySelectorAll('.overlay-container, .swipe-container, .diff-container').forEach(el => el.remove());

    // Reset to side-by-side layout
    comparison.classList.remove('mode-overlay', 'mode-swipe', 'mode-diff');
    const cColumn = comparison.querySelector('.c-output');
    const rustColumn = comparison.querySelector('.rust-output');
    // Reset visibility (keep in layout for height)
    cColumn.style.visibility = '';
    cColumn.style.pointerEvents = '';
    rustColumn.style.visibility = '';
    rustColumn.style.pointerEvents = '';

    switch (state.mode) {
      case MODES.SIDE_BY_SIDE:
        // Default view, nothing special needed
        break;

      case MODES.DIFFERENCE:
        renderDifferenceView(card, comparison, state);
        break;

      case MODES.ONION_SKIN:
        renderOnionSkinView(card, comparison, state);
        break;

      case MODES.SWIPE:
        renderSwipeView(card, comparison, state);
        break;
    }
  }

  function renderDifferenceView(card, comparison, state) {
    comparison.classList.add('mode-diff');

    const cColumn = comparison.querySelector('.c-output');
    const rustColumn = comparison.querySelector('.rust-output');
    // Hide but keep in layout for height
    cColumn.style.visibility = 'hidden';
    cColumn.style.pointerEvents = 'none';
    rustColumn.style.visibility = 'hidden';
    rustColumn.style.pointerEvents = 'none';

    const container = document.createElement('div');
    container.className = 'diff-container';
    container.innerHTML = `
      <div class="diff-header">
        <span class="diff-legend">
          <span class="legend-c">C only</span>
          <span class="legend-rust">Rust only</span>
          <span class="legend-both">Both</span>
        </span>
      </div>
      <div class="diff-canvas-container">
        <canvas class="diff-canvas"></canvas>
        <div class="diff-loading">Generating difference...</div>
      </div>
    `;
    comparison.appendChild(container);

    // Generate diff asynchronously
    generateDiff(state.cSvg, state.rustSvg, container.querySelector('.diff-canvas'),
      container.querySelector('.diff-loading'));
  }

  function renderOnionSkinView(card, comparison, state) {
    comparison.classList.add('mode-overlay');

    const cColumn = comparison.querySelector('.c-output');
    const rustColumn = comparison.querySelector('.rust-output');
    // Hide but keep in layout for height
    cColumn.style.visibility = 'hidden';
    cColumn.style.pointerEvents = 'none';
    rustColumn.style.visibility = 'hidden';
    rustColumn.style.pointerEvents = 'none';

    const container = document.createElement('div');
    container.className = 'overlay-container';

    // Clone the SVGs for overlay
    const cClone = state.cSvg.cloneNode(true);
    const rustClone = state.rustSvg.cloneNode(true);

    cClone.classList.add('overlay-c');
    rustClone.classList.add('overlay-rust');
    rustClone.style.opacity = state.opacity;

    // Add labels showing which is base vs overlay
    const labels = document.createElement('div');
    labels.className = 'overlay-labels';
    labels.innerHTML = `
      <span>C (base)</span>
      <span>Rust (${Math.round(state.opacity * 100)}%)</span>
    `;

    container.appendChild(cClone);
    container.appendChild(rustClone);
    container.appendChild(labels);
    comparison.appendChild(container);
  }

  function renderSwipeView(card, comparison, state) {
    comparison.classList.add('mode-swipe');

    const cColumn = comparison.querySelector('.c-output');
    const rustColumn = comparison.querySelector('.rust-output');
    // Hide but keep in layout for height
    cColumn.style.visibility = 'hidden';
    cColumn.style.pointerEvents = 'none';
    rustColumn.style.visibility = 'hidden';
    rustColumn.style.pointerEvents = 'none';

    const container = document.createElement('div');
    container.className = 'swipe-container';

    // Clone the SVGs
    const cClone = state.cSvg.cloneNode(true);
    const rustClone = state.rustSvg.cloneNode(true);

    container.innerHTML = `
      <div class="swipe-layer swipe-c"></div>
      <div class="swipe-layer swipe-rust"></div>
      <div class="swipe-divider" style="left: ${state.swipePosition}%">
        <div class="swipe-handle">
          <svg width="8" height="24" viewBox="0 0 8 24" fill="currentColor">
            <circle cx="4" cy="6" r="2"/>
            <circle cx="4" cy="12" r="2"/>
            <circle cx="4" cy="18" r="2"/>
          </svg>
        </div>
      </div>
      <div class="swipe-labels">
        <span class="label-c">C</span>
        <span class="label-rust">Rust</span>
      </div>
    `;

    container.querySelector('.swipe-c').appendChild(cClone);
    container.querySelector('.swipe-rust').appendChild(rustClone);

    // Set initial clip
    updateSwipeClip(container, state.swipePosition);

    // Add drag handling - can start drag from anywhere in container
    const divider = container.querySelector('.swipe-divider');
    let isDragging = false;

    // Start drag from container click (not just divider)
    container.addEventListener('mousedown', startDrag);
    container.addEventListener('touchstart', startDrag, { passive: true });

    function startDrag(e) {
      isDragging = true;
      // Immediately update position on click
      drag(e);
      document.addEventListener('mousemove', drag);
      document.addEventListener('mouseup', stopDrag);
      document.addEventListener('touchmove', drag, { passive: true });
      document.addEventListener('touchend', stopDrag);
    }

    function drag(e) {
      if (!isDragging) return;
      const rect = container.getBoundingClientRect();
      const clientX = e.touches ? e.touches[0].clientX : e.clientX;
      const x = clientX - rect.left;
      const percent = Math.max(0, Math.min(100, (x / rect.width) * 100));
      state.swipePosition = percent;
      divider.style.left = `${percent}%`;
      updateSwipeClip(container, percent);
    }

    function stopDrag() {
      isDragging = false;
      document.removeEventListener('mousemove', drag);
      document.removeEventListener('mouseup', stopDrag);
      document.removeEventListener('touchmove', drag);
      document.removeEventListener('touchend', stopDrag);
    }

    comparison.appendChild(container);
  }

  function updateSwipeClip(container, percent) {
    const rustLayer = container.querySelector('.swipe-rust');
    rustLayer.style.clipPath = `inset(0 0 0 ${percent}%)`;
  }

  async function generateDiff(cSvg, rustSvg, canvas, loadingEl) {
    try {
      // Render both SVGs to canvas
      const [cImg, rustImg] = await Promise.all([
        svgToImage(cSvg),
        svgToImage(rustSvg)
      ]);

      const width = Math.max(cImg.width, rustImg.width);
      const height = Math.max(cImg.height, rustImg.height);

      // Handle HiDPI displays
      const dpr = window.devicePixelRatio || 1;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = width + 'px';
      canvas.style.height = height + 'px';
      const ctx = canvas.getContext('2d');
      ctx.scale(dpr, dpr);

      // Draw C version
      const cCanvas = document.createElement('canvas');
      cCanvas.width = width;
      cCanvas.height = height;
      const cCtx = cCanvas.getContext('2d');
      cCtx.fillStyle = 'white';
      cCtx.fillRect(0, 0, width, height);
      cCtx.drawImage(cImg, 0, 0);
      const cData = cCtx.getImageData(0, 0, width, height);

      // Draw Rust version
      const rustCanvas = document.createElement('canvas');
      rustCanvas.width = width;
      rustCanvas.height = height;
      const rustCtx = rustCanvas.getContext('2d');
      rustCtx.fillStyle = 'white';
      rustCtx.fillRect(0, 0, width, height);
      rustCtx.drawImage(rustImg, 0, 0);
      const rustData = rustCtx.getImageData(0, 0, width, height);

      // Generate diff on a temp canvas (putImageData ignores transforms)
      const diffCanvas = document.createElement('canvas');
      diffCanvas.width = width;
      diffCanvas.height = height;
      const diffCtx = diffCanvas.getContext('2d');
      const diffData = diffCtx.createImageData(width, height);

      for (let i = 0; i < cData.data.length; i += 4) {
        const cGray = (cData.data[i] + cData.data[i+1] + cData.data[i+2]) / 3;
        const rustGray = (rustData.data[i] + rustData.data[i+1] + rustData.data[i+2]) / 3;

        const cPresent = cGray < 250; // Not white = has content
        const rustPresent = rustGray < 250;

        if (cPresent && rustPresent) {
          // Both have content - show in neutral gray/green
          diffData.data[i] = 100;
          diffData.data[i+1] = 150;
          diffData.data[i+2] = 100;
          diffData.data[i+3] = 255;
        } else if (cPresent) {
          // Only C has content - show in blue
          diffData.data[i] = 59;
          diffData.data[i+1] = 130;
          diffData.data[i+2] = 246;
          diffData.data[i+3] = 255;
        } else if (rustPresent) {
          // Only Rust has content - show in orange
          diffData.data[i] = 249;
          diffData.data[i+1] = 115;
          diffData.data[i+2] = 22;
          diffData.data[i+3] = 255;
        } else {
          // Neither has content - transparent
          diffData.data[i] = 0;
          diffData.data[i+1] = 0;
          diffData.data[i+2] = 0;
          diffData.data[i+3] = 0;
        }
      }

      // Put diff data to temp canvas, then draw scaled to output canvas
      diffCtx.putImageData(diffData, 0, 0);
      ctx.drawImage(diffCanvas, 0, 0);
      loadingEl.style.display = 'none';
    } catch (err) {
      console.error('Failed to generate diff:', err);
      loadingEl.textContent = 'Failed to generate diff';
    }
  }

  async function calculateSSIM(card) {
    const cardId = card.id;
    const state = cardStates.get(cardId);
    const ssimBtn = card.querySelector('.ssim-btn');
    const ssimValue = ssimBtn.querySelector('.ssim-value');

    if (!state.cSvg || !state.rustSvg) {
      ssimValue.textContent = 'N/A';
      return;
    }

    ssimValue.textContent = '...';

    try {
      const [cImg, rustImg] = await Promise.all([
        svgToImage(state.cSvg),
        svgToImage(state.rustSvg)
      ]);

      const ssim = computeSSIM(cImg, rustImg);
      state.ssim = ssim;
      ssimValue.textContent = ssim.toFixed(4);

      // Color code the result
      ssimBtn.classList.remove('ssim-good', 'ssim-ok', 'ssim-bad');
      if (ssim > 0.99) {
        ssimBtn.classList.add('ssim-good');
      } else if (ssim > 0.9) {
        ssimBtn.classList.add('ssim-ok');
      } else {
        ssimBtn.classList.add('ssim-bad');
      }
    } catch (err) {
      console.error('Failed to calculate SSIM:', err);
      ssimValue.textContent = 'ERR';
    }
  }

  function svgToImage(svg, targetWidth = 600) {
    return new Promise((resolve, reject) => {
      // Clone SVG and set explicit dimensions for proper rendering
      const clone = svg.cloneNode(true);
      const viewBox = clone.getAttribute('viewBox');

      if (viewBox) {
        const [, , vbWidth, vbHeight] = viewBox.split(/\s+/).map(Number);
        const aspectRatio = vbHeight / vbWidth;
        clone.setAttribute('width', targetWidth);
        clone.setAttribute('height', Math.round(targetWidth * aspectRatio));
      } else {
        // Fallback: set reasonable default size
        clone.setAttribute('width', targetWidth);
        clone.setAttribute('height', targetWidth);
      }

      const svgData = new XMLSerializer().serializeToString(clone);
      const svgBlob = new Blob([svgData], { type: 'image/svg+xml;charset=utf-8' });
      const url = URL.createObjectURL(svgBlob);

      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        resolve(img);
      };
      img.onerror = () => {
        URL.revokeObjectURL(url);
        reject(new Error('Failed to load SVG as image'));
      };
      img.src = url;
    });
  }

  function computeSSIM(img1, img2) {
    // Simplified SSIM implementation
    const width = Math.max(img1.width, img2.width);
    const height = Math.max(img1.height, img2.height);

    const canvas1 = document.createElement('canvas');
    canvas1.width = width;
    canvas1.height = height;
    const ctx1 = canvas1.getContext('2d');
    ctx1.fillStyle = 'white';
    ctx1.fillRect(0, 0, width, height);
    ctx1.drawImage(img1, 0, 0);

    const canvas2 = document.createElement('canvas');
    canvas2.width = width;
    canvas2.height = height;
    const ctx2 = canvas2.getContext('2d');
    ctx2.fillStyle = 'white';
    ctx2.fillRect(0, 0, width, height);
    ctx2.drawImage(img2, 0, 0);

    const data1 = ctx1.getImageData(0, 0, width, height).data;
    const data2 = ctx2.getImageData(0, 0, width, height).data;

    // Convert to grayscale and compute SSIM
    const n = width * height;
    let sum1 = 0, sum2 = 0, sum1Sq = 0, sum2Sq = 0, sum12 = 0;

    for (let i = 0; i < data1.length; i += 4) {
      const g1 = (data1[i] + data1[i+1] + data1[i+2]) / 3;
      const g2 = (data2[i] + data2[i+1] + data2[i+2]) / 3;
      sum1 += g1;
      sum2 += g2;
      sum1Sq += g1 * g1;
      sum2Sq += g2 * g2;
      sum12 += g1 * g2;
    }

    const mean1 = sum1 / n;
    const mean2 = sum2 / n;
    const var1 = sum1Sq / n - mean1 * mean1;
    const var2 = sum2Sq / n - mean2 * mean2;
    const covar = sum12 / n - mean1 * mean2;

    const c1 = 6.5025; // (0.01 * 255)^2
    const c2 = 58.5225; // (0.03 * 255)^2

    const ssim = ((2 * mean1 * mean2 + c1) * (2 * covar + c2)) /
                 ((mean1 * mean1 + mean2 * mean2 + c1) * (var1 + var2 + c2));

    return ssim;
  }

  function handleGlobalKeydown(e) {
    // Only handle if not in an input
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

    // Find the currently visible/focused card
    const visibleCard = findVisibleCard();
    if (!visibleCard) return;

    switch (e.key) {
      case '1':
        setMode(visibleCard, MODES.SIDE_BY_SIDE);
        break;
      case '2':
        setMode(visibleCard, MODES.DIFFERENCE);
        break;
      case '3':
        setMode(visibleCard, MODES.ONION_SKIN);
        break;
      case '4':
        setMode(visibleCard, MODES.SWIPE);
        break;
      case 's':
        if (!e.ctrlKey && !e.metaKey) {
          calculateSSIM(visibleCard);
        }
        break;
    }
  }

  function findVisibleCard() {
    const cards = document.querySelectorAll('.test-card');
    const viewportCenter = window.innerHeight / 2;

    let closestCard = null;
    let closestDistance = Infinity;

    cards.forEach(card => {
      const rect = card.getBoundingClientRect();
      const cardCenter = rect.top + rect.height / 2;
      const distance = Math.abs(cardCenter - viewportCenter);

      if (distance < closestDistance && rect.top < window.innerHeight && rect.bottom > 0) {
        closestDistance = distance;
        closestCard = card;
      }
    });

    return closestCard;
  }
})();
