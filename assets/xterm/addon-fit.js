!function(root, factory) {
  if (typeof exports === "object" && typeof module === "object") {
    module.exports = factory();
  } else if (typeof define === "function" && define.amd) {
    define([], factory);
  } else if (typeof exports === "object") {
    exports.FitAddon = factory();
  } else {
    root.FitAddon = factory();
  }
}(globalThis, function() {
  "use strict";

  const numberFromStyle = (style, property) => {
    const value = Number.parseFloat(style.getPropertyValue(property) || "0");
    return Number.isFinite(value) ? value : 0;
  };

  const elementSize = (element, style, property) => {
    const rect = element && element.getBoundingClientRect ? element.getBoundingClientRect() : null;
    const rectValue = rect ? Number(rect[property] || 0) : 0;
    if (Number.isFinite(rectValue) && rectValue > 0) {
      return rectValue;
    }
    const styleValue = Number.parseFloat(style.getPropertyValue(property) || "0");
    return Number.isFinite(styleValue) ? styleValue : 0;
  };

  class FitAddon {
    activate(terminal) {
      this._terminal = terminal;
    }

    dispose() {}

    fit() {
      const proposed = this.proposeDimensions();
      if (
        !proposed
        || !this._terminal
        || Number.isNaN(proposed.cols)
        || Number.isNaN(proposed.rows)
      ) {
        return;
      }
      const core = this._terminal._core;
      if (this._terminal.rows !== proposed.rows || this._terminal.cols !== proposed.cols) {
        const renderService = core
          ? core._renderService || core.renderService || null
          : null;
        if (renderService && typeof renderService.clear === "function") {
          renderService.clear();
        }
        this._terminal.resize(proposed.cols, proposed.rows);
      }
    }

    proposeDimensions() {
      if (!this._terminal) {
        return undefined;
      }
      const element = this._terminal.element;
      const parent = element ? element.parentElement : null;
      if (!element || !parent) {
        return undefined;
      }
      const dimensions = this._terminal._core._renderService.dimensions;
      if (dimensions.css.cell.width === 0 || dimensions.css.cell.height === 0) {
        return undefined;
      }
      const scrollbarWidth = this._terminal.options.scrollback === 0
        ? 0
        : this._terminal.options.overviewRuler?.width || 14;
      const parentStyle = window.getComputedStyle(parent);
      const elementStyle = window.getComputedStyle(element);
      const parentHeight = elementSize(parent, parentStyle, "height");
      const parentWidth = Math.max(0, elementSize(parent, parentStyle, "width"));
      const verticalPadding =
        numberFromStyle(elementStyle, "padding-top")
        + numberFromStyle(elementStyle, "padding-bottom");
      const horizontalPadding =
        numberFromStyle(elementStyle, "padding-right")
        + numberFromStyle(elementStyle, "padding-left");
      const bottomGuardPx = Math.max(0, Number(root.__yggtermXtermFitBottomGuardPx || 2));
      const availableHeight = Math.max(0, parentHeight - verticalPadding - bottomGuardPx);
      const availableWidth = Math.max(0, parentWidth - horizontalPadding - scrollbarWidth);
      return {
        cols: Math.max(2, Math.floor(availableWidth / dimensions.css.cell.width)),
        rows: Math.max(1, Math.floor(availableHeight / dimensions.css.cell.height)),
      };
    }
  }

  return { FitAddon };
});
