(function () {
  "use strict";

  const terminalEl = document.getElementById("terminal");
  const diagnosticsEl = document.getElementById("diagnostics");
  const rendererEl = document.getElementById("renderer");
  const themeEl = document.getElementById("theme");
  const colsEl = document.getElementById("cols");
  const rowsEl = document.getElementById("rows");

  const ESC = "\x1b";
  const CSI = `${ESC}[`;
  const PROMPT = "\u203a";

  const themes = {
    dark: {
      background: "#262a33",
      foreground: "#e5e5e5",
      cursor: "#f8f8f0",
      cursorAccent: "#262a33",
      selectionBackground: "rgba(102, 175, 233, 0.35)",
      black: "#000000",
      red: "#cd3131",
      green: "#0dbc79",
      yellow: "#e5e510",
      blue: "#2472c8",
      magenta: "#bc3fbc",
      cyan: "#11a8cd",
      white: "#e5e5e5",
      brightBlack: "#666666",
      brightWhite: "#ffffff",
    },
    light: {
      background: "#ffffff",
      foreground: "#1f2328",
      cursor: "#0451a5",
      cursorAccent: "#ffffff",
      selectionBackground: "rgba(9, 105, 218, 0.20)",
      black: "#24292f",
      red: "#a1260d",
      green: "#0c6428",
      yellow: "#7a4f00",
      blue: "#0451a5",
      magenta: "#6936aa",
      cyan: "#0e6570",
      white: "#57606a",
      brightBlack: "#6e7781",
      brightWhite: "#8c959f",
    },
  };

  let term = null;
  let fitAddon = null;
  let canvasAddon = null;
  let animationTimer = null;
  let renderCount = 0;
  let writeParsedCount = 0;
  let lastRender = null;

  function disposeTimer() {
    if (animationTimer !== null) {
      clearInterval(animationTimer);
      animationTimer = null;
    }
  }

  function terminalDimensions() {
    const cols = clampNumber(Number(colsEl.value), 40, 220, 120);
    const rows = clampNumber(Number(rowsEl.value), 12, 80, 36);
    colsEl.value = String(cols);
    rowsEl.value = String(rows);
    return { cols, rows };
  }

  function clampNumber(value, min, max, fallback) {
    if (!Number.isFinite(value)) {
      return fallback;
    }
    return Math.max(min, Math.min(max, Math.round(value)));
  }

  function rebuildTerminal() {
    disposeTimer();
    renderCount = 0;
    writeParsedCount = 0;
    lastRender = null;
    if (term) {
      term.dispose();
    }
    terminalEl.textContent = "";

    const { cols, rows } = terminalDimensions();
    const theme = themes[themeEl.value] || themes.dark;
    term = new Terminal({
      allowProposedApi: true,
      cols,
      rows,
      scrollback: 2000,
      fontFamily: "'JetBrains Mono', 'DejaVu Sans Mono', monospace",
      fontSize: 14,
      fontWeight: "400",
      fontWeightBold: "700",
      lineHeight: 1,
      letterSpacing: 0,
      cursorBlink: false,
      cursorStyle: "block",
      cursorWidth: 1,
      minimumContrastRatio: 1,
      scrollOnEraseInDisplay: true,
      theme,
      windowOptions: {
        getCellSizePixels: true,
        getWinSizeChars: true,
        getWinSizePixels: true,
      },
    });

    fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    if (rendererEl.value === "canvas") {
      canvasAddon = new CanvasAddon.CanvasAddon();
      term.loadAddon(canvasAddon);
    } else {
      canvasAddon = null;
    }

    term.open(terminalEl);
    term.onRender((event) => {
      renderCount += 1;
      lastRender = event;
      scheduleDiagnostics();
    });
    term.onWriteParsed(() => {
      writeParsedCount += 1;
      scheduleDiagnostics();
    });
    term.focus();
    fitToFrame();
    writeFixture("plainPrompt");
  }

  function fitToFrame() {
    if (!term || !fitAddon) {
      return;
    }
    fitAddon.fit();
    colsEl.value = String(term.cols);
    rowsEl.value = String(term.rows);
    updateDiagnostics();
  }

  function resetScreen() {
    disposeTimer();
    term.reset();
    term.write(`${CSI}2J${CSI}H`);
  }

  function writeFixture(name) {
    if (!term) {
      return;
    }
    if (name !== "workingAnimation") {
      disposeTimer();
    }
    fixtures[name]();
    updateDiagnostics();
  }

  function writeHeader(title) {
    term.write(`${CSI}1m${title}${CSI}22m\r\n\r\n`);
  }

  function fillWorkRows(count) {
    for (let i = 1; i <= count; i += 1) {
      const color = i % 3 === 0 ? "36" : i % 3 === 1 ? "32" : "37";
      term.write(`${CSI}${color}mrow ${String(i).padStart(2, "0")} ${CSI}0m`);
      term.write("remote task output and wrapped status text".padEnd(94, "."));
      term.write("\r\n");
    }
  }

  function writeCodexFooter(promptPayload) {
    term.write("\r\n");
    term.write(`${CSI}90mgpt-5.5 xhigh · ~/git/samplescripts${CSI}0m\r\n`);
    term.write(`${CSI}90mWorked for 1m 50s${CSI}0m\r\n`);
    term.write(promptPayload);
  }

  const fixtures = {
    plainPrompt() {
      resetScreen();
      writeHeader("Plain prompt fixture");
      fillWorkRows(12);
      term.write("\r\n");
      term.write(`${PROMPT} Use /skills to list available skills`);
    },

    truecolorPrompt() {
      resetScreen();
      writeHeader("Truecolor prompt background fixture");
      fillWorkRows(12);
      term.write("\r\n");
      term.write(
        `${CSI}48;2;63;69;82m${CSI}38;2;231;237;245m${PROMPT} Use /skills to list available skills${CSI}0m`
      );
    },

    jojoPayload() {
      resetScreen();
      writeHeader("Jojo payload fixture, default background clears");
      fillWorkRows(Math.max(0, term.rows - 9));
      term.write(`${PROMPT} Use /skills to list available skills`);
      const payload =
        `${CSI}?2026h` +
        `${CSI}58;2H${CSI}0m${CSI}49m${CSI}K` +
        `${CSI}59;2H${CSI}0m${CSI}49m${CSI}K` +
        `${CSI}60;39H${CSI}0m${CSI}49m${CSI}K` +
        `${CSI}61;2H${CSI}0m${CSI}49m${CSI}K` +
        `${CSI}62;35H${CSI}0m${CSI}49m${CSI}K` +
        `${CSI}39m${CSI}49m${CSI}0m${CSI}?25h${CSI}60;3H${CSI}?2026l`;
      term.write(payload);
    },

    resizeBork() {
      resetScreen();
      writeHeader("Resize partial repaint fixture");
      fillWorkRows(Math.max(0, term.rows - 8));
      writeCodexFooter(`${PROMPT} Use /skills to list available skills`);
      const nextCols = Math.max(82, Math.min(167, term.cols - 14));
      const nextRows = Math.max(24, term.rows - 4);
      term.resize(nextCols, nextRows);
      colsEl.value = String(nextCols);
      rowsEl.value = String(nextRows);
      const bottom = Math.max(1, nextRows - 3);
      term.write(
        `${CSI}?2026h` +
          `${CSI}${bottom};2H${CSI}0m${CSI}49m${CSI}K` +
          `${CSI}${bottom + 1};2H${CSI}0m${CSI}49m${CSI}K` +
          `${CSI}${bottom + 2};35H${CSI}0m${CSI}49m${CSI}K` +
          `${CSI}39m${CSI}49m${CSI}0m${CSI}?25h${CSI}${bottom + 1};3H${CSI}?2026l`
      );
    },

    fullRepaint() {
      resetScreen();
      const nextCols = clampNumber(Number(colsEl.value), 40, 220, 120);
      const nextRows = clampNumber(Number(rowsEl.value), 12, 80, 36);
      term.resize(nextCols, nextRows);
      writeHeader("Full repaint fixture");
      fillWorkRows(Math.max(0, term.rows - 8));
      writeCodexFooter(
        `${CSI}48;2;63;69;82m${CSI}38;2;231;237;245m${PROMPT} Use /skills to list available skills${CSI}0m`
      );
    },

    workingAnimation() {
      resetScreen();
      writeHeader("Inline animation fixture");
      fillWorkRows(8);
      let frame = 0;
      const words = ["Working", "Working.", "Working..", "Working..."];
      const colors = ["36", "96", "37", "90"];
      animationTimer = setInterval(() => {
        const word = words[frame % words.length];
        const color = colors[frame % colors.length];
        term.write(`\r${CSI}K${CSI}${color}m${word}${CSI}0m`);
        frame += 1;
      }, 80);
    },
  };

  let diagnosticsPending = false;
  function scheduleDiagnostics() {
    if (diagnosticsPending) {
      return;
    }
    diagnosticsPending = true;
    requestAnimationFrame(() => {
      diagnosticsPending = false;
      updateDiagnostics();
    });
  }

  function sampleCell(row, col) {
    const buffer = term.buffer.active;
    const line = buffer.getLine(buffer.baseY + row);
    const cell = line && line.getCell ? line.getCell(col) : null;
    if (!cell) {
      return null;
    }
    return {
      chars: method(cell, "getChars"),
      width: method(cell, "getWidth"),
      fgDefault: method(cell, "isFgDefault"),
      bgDefault: method(cell, "isBgDefault"),
      fgMode: method(cell, "getFgColorMode"),
      bgMode: method(cell, "getBgColorMode"),
      fgColor: method(cell, "getFgColor"),
      bgColor: method(cell, "getBgColor"),
      inverse: method(cell, "isInverse"),
      bold: method(cell, "isBold"),
      dim: method(cell, "isDim"),
    };
  }

  function method(target, name) {
    try {
      return typeof target[name] === "function" ? target[name]() : null;
    } catch (_error) {
      return null;
    }
  }

  function lineText(row) {
    const buffer = term.buffer.active;
    const line = buffer.getLine(buffer.baseY + row);
    return line ? line.translateToString(true) : "";
  }

  function updateDiagnostics() {
    if (!term) {
      diagnosticsEl.textContent = "waiting for terminal";
      return;
    }
    const buffer = term.buffer.active;
    const cursorRow = buffer.cursorY;
    const cursorCol = buffer.cursorX;
    const firstPromptCol = Math.max(0, lineText(cursorRow).indexOf(PROMPT));
    const sampleCol = firstPromptCol >= 0 ? firstPromptCol : cursorCol;
    const core = term._core;
    const dimensions = core && core._renderService ? core._renderService.dimensions : null;
    diagnosticsEl.textContent = JSON.stringify(
      {
        renderer: rendererEl.value,
        cols: term.cols,
        rows: term.rows,
        baseY: buffer.baseY,
        viewportY: buffer.viewportY,
        cursor: { x: cursorCol, y: cursorRow },
        cursorLine: lineText(cursorRow),
        promptCell: sampleCell(cursorRow, sampleCol),
        renderCount,
        writeParsedCount,
        lastRender,
        dimensions: dimensions
          ? {
              cssCell: dimensions.css.cell,
              cssCanvas: dimensions.css.canvas,
              deviceCell: dimensions.device.cell,
              deviceCanvas: dimensions.device.canvas,
            }
          : null,
      },
      null,
      2
    );
  }

  document.getElementById("rebuild").addEventListener("click", rebuildTerminal);
  document.getElementById("fit").addEventListener("click", fitToFrame);
  document.querySelectorAll("[data-fixture]").forEach((button) => {
    button.addEventListener("click", () => writeFixture(button.dataset.fixture));
  });
  rendererEl.addEventListener("change", rebuildTerminal);
  themeEl.addEventListener("change", rebuildTerminal);

  rebuildTerminal();
  window.yggtermXtermLab = {
    term: () => term,
    writeFixture,
    fitToFrame,
    rebuildTerminal,
  };
})();
