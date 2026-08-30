import 'package:flutter/material.dart';

enum ReaderLayoutImpact { paintOnly, repackOnly, reflow }

final class PageletReaderConfiguration {
  const PageletReaderConfiguration({
    this.viewportWidth = 600,
    this.viewportHeight = 800,
    this.marginStart = 32,
    this.marginEnd = 32,
    this.marginTop = 40,
    this.marginBottom = 40,
    this.fontSize = 16,
    this.fontFingerprint = 0,
    this.letterSpacing = 0,
    this.lineHeight = 1,
    this.textColor = Colors.black,
    this.backgroundColor = Colors.white,
  });

  final double viewportWidth;
  final double viewportHeight;
  final double marginStart;
  final double marginEnd;
  final double marginTop;
  final double marginBottom;
  // ponytail: native layout options do not expose reader font overrides yet;
  // these values classify invalidation until that ABI exists.
  final double fontSize;
  final int fontFingerprint;
  final double letterSpacing;
  final double lineHeight;
  final Color textColor;
  final Color backgroundColor;

  ReaderLayoutImpact impactFrom(PageletReaderConfiguration previous) {
    if (viewportWidth != previous.viewportWidth ||
        marginStart != previous.marginStart ||
        marginEnd != previous.marginEnd ||
        fontSize != previous.fontSize ||
        fontFingerprint != previous.fontFingerprint ||
        letterSpacing != previous.letterSpacing ||
        lineHeight != previous.lineHeight) {
      return ReaderLayoutImpact.reflow;
    }
    if (viewportHeight != previous.viewportHeight ||
        marginTop != previous.marginTop ||
        marginBottom != previous.marginBottom) {
      return ReaderLayoutImpact.repackOnly;
    }
    return ReaderLayoutImpact.paintOnly;
  }
}
