// Cell-aware xterm snapshot serialization.
//
// The old cache used BufferLine.translateToString(true), which is useful for
// diagnostics but intentionally discards SGR foreground/background and text
// attributes. A cached reveal therefore came back as a lifeless text-only
// frame. This module keeps the plain text projection for counts/debugging and
// adds an ANSI projection that xterm can parse back into the same cell styles.

(function installTerminalFrameCache(root) {
  'use strict';

  const ESC = '\x1b[';

  function callCell(cell, method, fallback) {
    try {
      return cell && typeof cell[method] === 'function' ? cell[method]() : fallback;
    } catch (_error) {
      return fallback;
    }
  }

  function rgbComponents(value) {
    const color = Number(value) >>> 0;
    return [
      (color >>> 16) & 0xff,
      (color >>> 8) & 0xff,
      color & 0xff,
    ];
  }

  function cellStyle(cell) {
    const fgDefault = Boolean(callCell(cell, 'isFgDefault', true));
    const bgDefault = Boolean(callCell(cell, 'isBgDefault', true));
    const fgRgb = Boolean(callCell(cell, 'isFgRGB', false));
    const bgRgb = Boolean(callCell(cell, 'isBgRGB', false));
    const fgPalette = Boolean(callCell(cell, 'isFgPalette', false));
    const bgPalette = Boolean(callCell(cell, 'isBgPalette', false));
    const attributes = [
      Boolean(callCell(cell, 'isBold', false)),
      Boolean(callCell(cell, 'isDim', false)),
      Boolean(callCell(cell, 'isItalic', false)),
      Boolean(callCell(cell, 'isUnderline', false)),
      Boolean(callCell(cell, 'isBlink', false)),
      Boolean(callCell(cell, 'isInverse', false)),
      Boolean(callCell(cell, 'isInvisible', false)),
      Boolean(callCell(cell, 'isStrikethrough', false)),
    ];
    const fg = fgDefault
      ? ['default']
      : fgRgb
        ? ['rgb', Number(callCell(cell, 'getFgColor', 0)) >>> 0]
        : fgPalette
          ? ['palette', Number(callCell(cell, 'getFgColor', 0))]
          : ['indexed', Number(callCell(cell, 'getFgColor', 0))];
    const bg = bgDefault
      ? ['default']
      : bgRgb
        ? ['rgb', Number(callCell(cell, 'getBgColor', 0)) >>> 0]
        : bgPalette
          ? ['palette', Number(callCell(cell, 'getBgColor', 0))]
          : ['indexed', Number(callCell(cell, 'getBgColor', 0))];
    return {
      fg,
      bg,
      attributes,
      hasColor: !fgDefault || !bgDefault,
      hasAttributes: attributes.some(Boolean),
    };
  }

  function styleKey(style) {
    return JSON.stringify(style);
  }

  function sgrForStyle(style) {
    const codes = [0];
    const attributes = style.attributes;
    if (attributes[0]) codes.push(1);
    if (attributes[1]) codes.push(2);
    if (attributes[2]) codes.push(3);
    if (attributes[3]) codes.push(4);
    if (attributes[4]) codes.push(5);
    if (attributes[5]) codes.push(7);
    if (attributes[6]) codes.push(8);
    if (attributes[7]) codes.push(9);
    if (style.fg[0] === 'rgb') {
      const [r, g, b] = rgbComponents(style.fg[1]);
      codes.push(38, 2, r, g, b);
    } else if (style.fg[0] === 'palette' || style.fg[0] === 'indexed') {
      codes.push(38, 5, Math.max(0, Math.min(255, Number(style.fg[1]) || 0)));
    }
    if (style.bg[0] === 'rgb') {
      const [r, g, b] = rgbComponents(style.bg[1]);
      codes.push(48, 2, r, g, b);
    } else if (style.bg[0] === 'palette' || style.bg[0] === 'indexed') {
      codes.push(48, 5, Math.max(0, Math.min(255, Number(style.bg[1]) || 0)));
    }
    return `${ESC}${codes.join(';')}m`;
  }

  function serializeBuffer(term, maxRows) {
    try {
      const buffer = term && term.buffer && term.buffer.active;
      if (!buffer || typeof buffer.getLine !== 'function') return null;
      const length = Math.max(0, Number(buffer.length || 0));
      const rows = Math.max(1, Number(term.rows || 1));
      const rowLimit = Math.min(length, Math.max(300, Number(maxRows || rows * 8)));
      const start = Math.max(0, length - rowLimit);
      const visualLines = [];
      const logicalLines = [];
      const logicalAnsiLines = [];
      let hasAttributes = false;
      let colorCellCount = 0;
      let attributeCellCount = 0;
      let plainCurrent = '';
      let ansiCurrent = '';
      let logicalLineStarted = false;
      let currentStyleKey = null;
      let currentStyle = null;

      const finishLogicalLine = () => {
        if (currentStyleKey !== null) {
          ansiCurrent += `${ESC}0m`;
          currentStyleKey = null;
          currentStyle = null;
        }
        logicalLines.push(plainCurrent);
        logicalAnsiLines.push(ansiCurrent);
        plainCurrent = '';
        ansiCurrent = '';
        logicalLineStarted = false;
      };

      for (let row = start; row < length; row += 1) {
        const line = buffer.getLine(row);
        if (!(line && line.isWrapped) && logicalLineStarted) {
          finishLogicalLine();
        }
        const lineText = line && typeof line.translateToString === 'function'
          ? String(line.translateToString(true) || '')
          : '';
        visualLines.push(lineText);
        const columns = Math.max(
          0,
          Number(line && line.length != null ? line.length : term.cols || 0),
        );
        let visualAnsi = '';
        let visualPlain = '';
        let lastMeaningfulColumn = -1;
        const cells = [];
        for (let column = 0; column < columns; column += 1) {
          const cell = line && typeof line.getCell === 'function' ? line.getCell(column) : null;
          const chars = String(callCell(cell, 'getChars', '') || '');
          const width = Math.max(0, Number(callCell(cell, 'getWidth', 1) || 0));
          const style = cellStyle(cell);
          const meaningful = chars.length > 0 || style.hasColor || style.hasAttributes;
          if (meaningful) lastMeaningfulColumn = column;
          if (style.hasColor) colorCellCount += 1;
          if (style.hasAttributes) attributeCellCount += 1;
          hasAttributes = hasAttributes || style.hasColor || style.hasAttributes;
          cells.push({ chars, width, style });
        }
        for (let column = 0; column <= lastMeaningfulColumn; column += 1) {
          const entry = cells[column];
          if (!entry || entry.width === 0) continue;
          const text = entry.chars || ' '.repeat(Math.max(1, entry.width));
          visualPlain += text;
          const key = styleKey(entry.style);
          if (entry.style.hasColor || entry.style.hasAttributes) {
            if (key !== currentStyleKey) {
              visualAnsi += sgrForStyle(entry.style);
              currentStyleKey = key;
              currentStyle = entry.style;
            }
          } else if (currentStyleKey !== null) {
            visualAnsi += `${ESC}0m`;
            currentStyleKey = null;
            currentStyle = null;
          }
          if (entry.style.hasColor || entry.style.hasAttributes) {
            // The local style is only used to compare transitions; keeping it
            // explicit makes the serializer resilient to CellData reuse by
            // BufferLine.getCell().
            currentStyle = entry.style;
          }
          visualAnsi += text;
        }
        plainCurrent += visualPlain;
        ansiCurrent += visualAnsi;
        logicalLineStarted = true;
      }
      if (logicalLineStarted || logicalLines.length === 0) {
        finishLogicalLine();
      }
      const text = logicalLines.join('\r\n');
      const ansiText = logicalAnsiLines.join('\r\n');
      return {
        text,
        ansiText,
        visualLineCount: visualLines.length,
        logicalLineCount: logicalLines.length,
        nonblankLineCount: logicalLines
          .filter((line) => String(line || '').trim().length > 0)
          .length,
        hasAttributes,
        colorCellCount,
        attributeCellCount,
      };
    } catch (_error) {
      return null;
    }
  }

  const api = { serializeBuffer };
  if (root) root.__yggtermTerminalFrameCache = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
