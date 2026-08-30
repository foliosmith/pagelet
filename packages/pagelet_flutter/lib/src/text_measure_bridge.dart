import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:characters/characters.dart';
import 'package:flutter/painting.dart';

const int _layoutUnitScale = 64;
const int _currentSchemaVersion = 3;
const int _measurePayloadKind = 2;
const int _measuredPayloadKind = 3;
const int _headerLength = 20;
const int _maximumPayloadBytes = 64 * 1024 * 1024;
const int _maximumCollectionItems = 1000000;
const int _maximumStringBytes = 8 * 1024 * 1024;
const int _minimumInt64 = -0x8000000000000000;
const int _maximumInt64 = 0x7fffffffffffffff;
const int _fnvOffsetBasis = 0xcbf29ce484222325;
const int _fnvPrime = 0x100000001b3;
const List<int> _wireMagic = <int>[
  0x50,
  0x47,
  0x4c,
  0x54,
  0x53,
  0x43,
  0x4e,
  0x00,
];

/// Stable backend identity used by the Flutter host paragraph measurer.
const int pageletFlutterTextBackendId = 0x666c757474657201;

/// Measures every paragraph in one versioned pagelet `MeasureBatch`.
///
/// One call consumes the bytes returned by `pagelet_layout_request`, lays out
/// every requested paragraph locally with Flutter, and produces one complete
/// `MeasuredBatch` for `pagelet_layout_submit_measurements`. No per-line native
/// callback is made.
final class TextMeasureBridge {
  /// Creates a bridge for a concrete host font environment.
  ///
  /// [fontFingerprint] must change whenever the registered font bytes or font
  /// resolution policy changes so native layout caches cannot reuse stale
  /// measurements.
  const TextMeasureBridge({
    required this.fontFingerprint,
    this.backendId = pageletFlutterTextBackendId,
  });

  /// Stable identity for this Flutter measurement implementation.
  final int backendId;

  /// Stable identity for the fonts visible to Flutter.
  final int fontFingerprint;

  /// Decodes, validates, and measures a complete native request batch.
  TextMeasurementBatch measureBatch(Uint8List wireBytes) {
    _requireUint64(backendId, 'backendId');
    _requireUint64(fontFingerprint, 'fontFingerprint');
    final requestBatch = _MeasureWireCodec.decodeRequest(wireBytes);
    final paragraphs = <int, MeasuredParagraph>{};
    final results = <_MeasuredText>[];
    try {
      for (final request in requestBatch.requests) {
        if (paragraphs.containsKey(request.paragraphId)) {
          throw const FormatException(
            'MeasureBatch contains duplicate paragraph ids.',
          );
        }
        final measured = _measure(request);
        paragraphs[request.paragraphId] = measured.paragraph;
        results.add(measured.result);
      }
      final measuredBytes = _MeasureWireCodec.encodeMeasured(
        schemaVersion: requestBatch.schemaVersion,
        backendId: backendId,
        fontFingerprint: fontFingerprint,
        results: results,
      );
      return TextMeasurementBatch._(measuredBytes, paragraphs);
    } catch (_) {
      for (final paragraph in paragraphs.values) {
        paragraph.dispose();
      }
      rethrow;
    }
  }

  _MeasuredParagraphResult _measure(_MeasureRequest request) {
    final offsets = _TextOffsets(request.text);
    offsets.requireByteBoundary(request.textStart, 'measure text range start');
    offsets.requireByteBoundary(request.textEnd, 'measure text range end');
    if (request.textStart > request.textEnd) {
      throw const FormatException('Measure text range is reversed.');
    }
    final rangeStart = offsets.utf16ForByte(request.textStart);
    final rangeEnd = offsets.utf16ForByte(request.textEnd);
    final measuredText = request.text.substring(rangeStart, rangeEnd);
    final measuredOffsets = _TextOffsets(measuredText);
    final span = _buildSpan(request, offsets, rangeStart, rangeEnd);
    final direction = _flutterDirection(request.direction, measuredText);
    final textScale = _layoutUnitToDouble(request.textScale);
    if (!textScale.isFinite || textScale <= 0) {
      throw const FormatException(
        'MeasureBatch contains an invalid text scale.',
      );
    }
    final width = _layoutUnitToDouble(request.availableWidth);
    if (!width.isFinite || width < 0) {
      throw const FormatException(
        'MeasureBatch contains an invalid available width.',
      );
    }
    final painter = TextPainter(
      text: span,
      textDirection: direction,
      textScaler: TextScaler.linear(textScale),
      locale: _locale(request.locale),
      strutStyle: _strutStyle(request),
      textHeightBehavior: _heightBehavior(request.heightBehavior),
    );
    try {
      painter.layout(maxWidth: width);
      final flutterLines = painter.computeLineMetrics();
      final lineRanges = _lineRanges(painter, measuredText, flutterLines);
      final lines = <_MeasuredLine>[];
      for (var index = 0; index < flutterLines.length; index += 1) {
        final metrics = flutterLines[index];
        final range = lineRanges[index];
        final boxes = range.start == range.end
            ? const <TextBox>[]
            : painter.getBoxesForSelection(
                TextSelection(
                  baseOffset: range.start,
                  extentOffset: range.end,
                ),
                boxHeightStyle: ui.BoxHeightStyle.tight,
                boxWidthStyle: ui.BoxWidthStyle.tight,
              );
        final lineTop = metrics.baseline - metrics.ascent;
        final inkBounds = _inkBounds(boxes, metrics.left, lineTop);
        lines.add(
          _MeasuredLine(
            textStart: measuredOffsets.byteForUtf16(range.start),
            textEnd: measuredOffsets.byteForUtf16(range.end),
            baseline: _doubleToLayoutUnit(metrics.baseline - lineTop),
            ascent: _doubleToLayoutUnit(metrics.ascent),
            descent: _doubleToLayoutUnit(metrics.descent),
            lineHeight: _doubleToLayoutUnit(metrics.height),
            width: _doubleToLayoutUnit(metrics.width),
            inkX: _doubleToLayoutUnit(inkBounds.left),
            inkY: _doubleToLayoutUnit(inkBounds.top),
            inkWidth: _doubleToLayoutUnit(inkBounds.width),
            inkHeight: _doubleToLayoutUnit(inkBounds.height),
            hardBreak: range.hardBreak,
          ),
        );
      }
      final clusters = _clusters(
        painter,
        measuredText,
        measuredOffsets,
        flutterLines,
      );
      final measuredWidth = flutterLines.fold<double>(
        0,
        (current, line) => line.width > current ? line.width : current,
      );
      final result = _MeasuredText(
        requestId: request.id,
        requestFingerprint: request.requestFingerprint,
        width: _doubleToLayoutUnit(measuredWidth),
        height: _doubleToLayoutUnit(painter.height),
        utf8Length: measuredOffsets.utf8Length,
        lines: lines,
        clusters: clusters,
        measurementFingerprint: _measurementFingerprint(
          requestFingerprint: request.requestFingerprint,
          backendId: backendId,
          fontFingerprint: fontFingerprint,
          width: _doubleToLayoutUnit(measuredWidth),
          height: _doubleToLayoutUnit(painter.height),
          utf8Length: measuredOffsets.utf8Length,
          lines: lines,
          clusters: clusters,
        ),
      );
      return _MeasuredParagraphResult(
        MeasuredParagraph._(
          paragraphId: request.paragraphId,
          requestFingerprint: request.requestFingerprint,
          text: measuredText,
          painter: painter,
          measurementFingerprint: result.measurementFingerprint,
        ),
        result,
      );
    } catch (_) {
      painter.dispose();
      rethrow;
    }
  }
}

/// One completed batch ready for a single native measurement submission.
///
/// The measured [paragraphs] retain the exact [TextPainter] instances used to
/// produce the metrics. Keep this object alive while the corresponding page
/// scenes are rendered, then call [dispose].
final class TextMeasurementBatch {
  TextMeasurementBatch._(
    Uint8List wireBytes,
    Map<int, MeasuredParagraph> paragraphs,
  )   : _wireBytes = Uint8List.fromList(wireBytes),
        paragraphs = Map<int, MeasuredParagraph>.unmodifiable(paragraphs);

  /// Encoded `MeasuredBatch` accepted by pagelet's C ABI.
  Uint8List get wireBytes => Uint8List.fromList(_wireBytes);

  final Uint8List _wireBytes;

  /// Measured paragraph cache keyed by the native paragraph id.
  final Map<int, MeasuredParagraph> paragraphs;

  bool _isDisposed = false;

  /// Whether the retained Flutter paragraph resources have been released.
  bool get isDisposed => _isDisposed;

  /// Releases every retained [TextPainter]. Repeated calls are safe.
  void dispose() {
    if (_isDisposed) {
      return;
    }
    _isDisposed = true;
    for (final paragraph in paragraphs.values) {
      paragraph.dispose();
    }
  }
}

/// A Flutter paragraph measured with the same parameters sent back to pagelet.
final class MeasuredParagraph {
  MeasuredParagraph._({
    required this.paragraphId,
    required this.requestFingerprint,
    required this.measurementFingerprint,
    required this.text,
    required this.painter,
  });

  /// Stable paragraph identity used by the future page-scene decoder.
  final int paragraphId;

  /// Fingerprint copied from the native measurement request.
  final int requestFingerprint;

  /// Fingerprint written to the native measurement response.
  final int measurementFingerprint;

  /// Exact substring represented by this paragraph.
  final String text;

  /// Painter used to generate the returned line and cluster metrics.
  final TextPainter painter;

  bool _isDisposed = false;

  /// Whether [painter] has been disposed.
  bool get isDisposed => _isDisposed;

  /// Releases the paragraph's native Flutter resources. Repeated calls are safe.
  void dispose() {
    if (_isDisposed) {
      return;
    }
    _isDisposed = true;
    painter.dispose();
  }
}

final class _MeasuredParagraphResult {
  const _MeasuredParagraphResult(this.paragraph, this.result);

  final MeasuredParagraph paragraph;
  final _MeasuredText result;
}

TextSpan _buildSpan(
  _MeasureRequest request,
  _TextOffsets offsets,
  int rangeStart,
  int rangeEnd,
) {
  final baseStyle = _textStyle(
    request.fontSize,
    0,
    request.fontCandidates,
  );
  final children = <InlineSpan>[];
  var cursor = rangeStart;
  final runs = request.styleRuns.toList(growable: false)
    ..sort((first, second) => first.start.compareTo(second.start));
  for (final run in runs) {
    offsets.requireByteBoundary(run.start, 'text style run start');
    offsets.requireByteBoundary(run.end, 'text style run end');
    if (run.start > run.end ||
        run.start < request.textStart ||
        run.end > request.textEnd) {
      throw const FormatException(
          'MeasureBatch contains an invalid style run.');
    }
    final start = offsets.utf16ForByte(run.start);
    final end = offsets.utf16ForByte(run.end);
    if (start < cursor) {
      throw const FormatException(
          'MeasureBatch style runs overlap or are unordered.');
    }
    if (cursor < start) {
      children.add(
        TextSpan(text: request.text.substring(cursor, start), style: baseStyle),
      );
    }
    if (start < end) {
      children.add(
        TextSpan(
          text: request.text.substring(start, end),
          style: _textStyle(run.fontSize, run.letterSpacing, run.fonts),
        ),
      );
    }
    cursor = end;
  }
  if (cursor < rangeEnd) {
    children.add(
      TextSpan(
          text: request.text.substring(cursor, rangeEnd), style: baseStyle),
    );
  }
  return TextSpan(style: baseStyle, children: children);
}

TextStyle _textStyle(
  int fontSize,
  int letterSpacing,
  _FontFallbackChain fonts,
) {
  final size = _layoutUnitToDouble(fontSize);
  if (!size.isFinite || size <= 0) {
    throw const FormatException('MeasureBatch contains an invalid font size.');
  }
  final spacing = _layoutUnitToDouble(letterSpacing);
  if (!spacing.isFinite) {
    throw const FormatException(
      'MeasureBatch contains invalid letter spacing.',
    );
  }
  final descriptor = fonts.primary;
  return TextStyle(
    fontFamily: descriptor.family,
    fontFamilyFallback: fonts.fallbacks
        .map((fallback) => fallback.family)
        .toList(growable: false),
    fontSize: size,
    letterSpacing: spacing,
    fontWeight: _fontWeight(descriptor.weight),
    fontStyle: descriptor.style == _FontStyle.normal
        ? FontStyle.normal
        : FontStyle.italic,
    fontVariations: descriptor.stretch == 100
        ? null
        : <ui.FontVariation>[
            ui.FontVariation('wdth', descriptor.stretch.toDouble()),
          ],
  );
}

FontWeight _fontWeight(int value) {
  final clamped = value.clamp(1, 1000);
  final index = ((clamped + 50) ~/ 100 - 1).clamp(0, 8);
  return FontWeight.values[index];
}

TextDirection _flutterDirection(_TextDirection direction, String text) {
  switch (direction) {
    case _TextDirection.ltr:
      return TextDirection.ltr;
    case _TextDirection.rtl:
      return TextDirection.rtl;
    case _TextDirection.auto:
      return _detectRtl(text) ? TextDirection.rtl : TextDirection.ltr;
  }
}

bool _detectRtl(String text) {
  for (final rune in text.runes) {
    if ((rune >= 0x0590 && rune <= 0x08ff) ||
        (rune >= 0xfb1d && rune <= 0xfdff) ||
        (rune >= 0xfe70 && rune <= 0xfeff)) {
      return true;
    }
    if ((rune >= 0x0041 && rune <= 0x005a) ||
        (rune >= 0x0061 && rune <= 0x007a) ||
        (rune >= 0x00c0 && rune <= 0x02af) ||
        (rune >= 0x3040 && rune <= 0x30ff) ||
        (rune >= 0x3400 && rune <= 0x9fff)) {
      return false;
    }
  }
  return false;
}

Locale _locale(String tag) {
  if (tag == 'und') {
    return const Locale('und');
  }
  final parts = tag.split(RegExp('[-_]'));
  final language = parts.first;
  if (language.isEmpty ||
      !RegExp(r'^[A-Za-z]{2,8}$').hasMatch(language) ||
      parts.any((part) =>
          part.isEmpty || !RegExp(r'^[A-Za-z0-9]{1,8}$').hasMatch(part))) {
    throw const FormatException('MeasureBatch contains an invalid locale.');
  }
  var index = 1;
  String? scriptCode;
  String? countryCode;
  if (index < parts.length && parts[index].length == 4) {
    scriptCode = parts[index];
    index += 1;
  }
  if (index < parts.length &&
      (parts[index].length == 2 || parts[index].length == 3)) {
    countryCode = parts[index];
  }
  return Locale.fromSubtags(
    languageCode: language,
    scriptCode: scriptCode,
    countryCode: countryCode,
  );
}

StrutStyle? _strutStyle(_MeasureRequest request) {
  if (request.heightBehavior != _HeightBehavior.includeStrut) {
    return null;
  }
  final ascent = _layoutUnitToDouble(request.strutAscent);
  final descent = _layoutUnitToDouble(request.strutDescent);
  final leading = _layoutUnitToDouble(request.strutLeading);
  if (!ascent.isFinite ||
      !descent.isFinite ||
      !leading.isFinite ||
      ascent < 0 ||
      descent < 0 ||
      leading < 0) {
    throw const FormatException('MeasureBatch contains invalid strut metrics.');
  }
  final fontSize = ascent + descent;
  if (fontSize <= 0) {
    return null;
  }
  return StrutStyle(
    fontFamily: request.fontCandidates.primary.family,
    fontFamilyFallback: request.fontCandidates.fallbacks
        .map((fallback) => fallback.family)
        .toList(growable: false),
    fontSize: fontSize,
    height: 1,
    leading: leading / fontSize,
  );
}

ui.TextHeightBehavior? _heightBehavior(_HeightBehavior behavior) {
  switch (behavior) {
    case _HeightBehavior.natural:
    case _HeightBehavior.includeStrut:
      return null;
    case _HeightBehavior.tight:
      return const ui.TextHeightBehavior(
        applyHeightToFirstAscent: false,
        applyHeightToLastDescent: false,
      );
  }
}

List<_LineRange> _lineRanges(
  TextPainter painter,
  String text,
  List<ui.LineMetrics> metrics,
) {
  if (metrics.isEmpty) {
    return const <_LineRange>[];
  }
  final ranges = <_LineRange>[];
  var cursor = 0;
  for (var index = 0; index < metrics.length; index += 1) {
    if (text.isEmpty || cursor == text.length) {
      ranges.add(_LineRange(cursor, cursor, false));
      continue;
    }
    final boundary = painter.getLineBoundary(
      TextPosition(offset: cursor, affinity: TextAffinity.downstream),
    );
    final start = boundary.start.clamp(cursor, text.length);
    final end = boundary.end.clamp(start, text.length);
    final breakLength = _hardBreakLengthAt(text, end);
    ranges.add(_LineRange(start, end, breakLength > 0));
    cursor = (end + breakLength).clamp(0, text.length);
  }
  return ranges;
}

int _hardBreakLengthAt(String text, int offset) {
  if (offset >= text.length) {
    return 0;
  }
  final first = text.codeUnitAt(offset);
  if (first == 0x0d) {
    return offset + 1 < text.length && text.codeUnitAt(offset + 1) == 0x0a
        ? 2
        : 1;
  }
  return first == 0x0a ? 1 : 0;
}

ui.Rect _inkBounds(List<TextBox> boxes, double lineLeft, double lineTop) {
  if (boxes.isEmpty) {
    return ui.Rect.zero;
  }
  var left = boxes.first.left;
  var top = boxes.first.top;
  var right = boxes.first.right;
  var bottom = boxes.first.bottom;
  for (final box in boxes.skip(1)) {
    left = box.left < left ? box.left : left;
    top = box.top < top ? box.top : top;
    right = box.right > right ? box.right : right;
    bottom = box.bottom > bottom ? box.bottom : bottom;
  }
  return ui.Rect.fromLTRB(
    left - lineLeft,
    top - lineTop,
    right - lineLeft,
    bottom - lineTop,
  );
}

List<_MeasuredCluster> _clusters(
  TextPainter painter,
  String text,
  _TextOffsets offsets,
  List<ui.LineMetrics> lines,
) {
  final clusters = <_MeasuredCluster>[];
  var utf16Start = 0;
  for (final grapheme in text.characters) {
    final utf16End = utf16Start + grapheme.length;
    if (_hardBreakLengthAt(text, utf16Start) > 0) {
      utf16Start = utf16End;
      continue;
    }
    final boxes = painter.getBoxesForSelection(
      TextSelection(baseOffset: utf16Start, extentOffset: utf16End),
      boxHeightStyle: ui.BoxHeightStyle.tight,
      boxWidthStyle: ui.BoxWidthStyle.tight,
    );
    if (boxes.isNotEmpty) {
      var left = boxes.first.left;
      var right = boxes.first.right;
      var top = boxes.first.top;
      var bottom = boxes.first.bottom;
      for (final box in boxes.skip(1)) {
        left = box.left < left ? box.left : left;
        right = box.right > right ? box.right : right;
        top = box.top < top ? box.top : top;
        bottom = box.bottom > bottom ? box.bottom : bottom;
      }
      final lineIndex = _lineIndexForY(lines, (top + bottom) / 2);
      final lineLeft = lines.isEmpty ? 0.0 : lines[lineIndex].left;
      final xStart = left - lineLeft;
      final xEnd = right - lineLeft;
      clusters.add(
        _MeasuredCluster(
          textStart: offsets.byteForUtf16(utf16Start),
          textEnd: offsets.byteForUtf16(utf16End),
          lineIndex: lineIndex,
          xStart: _doubleToLayoutUnit(xStart < xEnd ? xStart : xEnd),
          xEnd: _doubleToLayoutUnit(xStart < xEnd ? xEnd : xStart),
        ),
      );
    }
    utf16Start = utf16End;
  }
  clusters.sort((first, second) {
    final byLine = first.lineIndex.compareTo(second.lineIndex);
    return byLine != 0 ? byLine : first.xStart.compareTo(second.xStart);
  });
  return clusters;
}

int _lineIndexForY(List<ui.LineMetrics> lines, double y) {
  if (lines.isEmpty) {
    return 0;
  }
  for (var index = 0; index < lines.length; index += 1) {
    final line = lines[index];
    final top = line.baseline - line.ascent;
    if (y <= top + line.height) {
      return index;
    }
  }
  return lines.length - 1;
}

int _measurementFingerprint({
  required int requestFingerprint,
  required int backendId,
  required int fontFingerprint,
  required int width,
  required int height,
  required int utf8Length,
  required List<_MeasuredLine> lines,
  required List<_MeasuredCluster> clusters,
}) {
  var hash = _fnvOffsetBasis;
  void add(int value) {
    var remaining = value;
    for (var index = 0; index < 8; index += 1) {
      hash = ((hash ^ (remaining & 0xff)) * _fnvPrime).toSigned(64);
      remaining >>= 8;
    }
  }

  add(requestFingerprint);
  add(backendId);
  add(fontFingerprint);
  add(width);
  add(height);
  add(utf8Length);
  add(lines.length);
  for (final line in lines) {
    add(line.textStart);
    add(line.textEnd);
    add(line.baseline);
    add(line.ascent);
    add(line.descent);
    add(line.lineHeight);
    add(line.width);
    add(line.inkX);
    add(line.inkY);
    add(line.inkWidth);
    add(line.inkHeight);
    add(line.hardBreak ? 1 : 0);
  }
  add(clusters.length);
  for (final cluster in clusters) {
    add(cluster.textStart);
    add(cluster.textEnd);
    add(cluster.lineIndex);
    add(cluster.xStart);
    add(cluster.xEnd);
  }
  return hash;
}

double _layoutUnitToDouble(int value) => value / _layoutUnitScale;

int _doubleToLayoutUnit(double value) {
  if (!value.isFinite) {
    throw const FormatException('Flutter produced non-finite text metrics.');
  }
  final scaled = (value * _layoutUnitScale).round();
  if (scaled < _minimumInt64 || scaled > _maximumInt64) {
    throw const FormatException(
        'Flutter text metrics exceed LayoutUnit range.');
  }
  return scaled;
}

void _requireUint64(int value, String field) {
  if (value < _minimumInt64 || value > _maximumInt64) {
    throw ArgumentError.value(
      value,
      field,
      'must fit the signed Dart representation of a 64-bit wire value',
    );
  }
}

final class _TextOffsets {
  _TextOffsets(String text) {
    var byteOffset = 0;
    var utf16Offset = 0;
    _byteToUtf16[0] = 0;
    _utf16ToByte[0] = 0;
    for (final rune in text.runes) {
      byteOffset += _utf8Length(rune);
      utf16Offset += rune > 0xffff ? 2 : 1;
      _byteToUtf16[byteOffset] = utf16Offset;
      _utf16ToByte[utf16Offset] = byteOffset;
    }
    utf8Length = byteOffset;
  }

  final Map<int, int> _byteToUtf16 = <int, int>{};
  final Map<int, int> _utf16ToByte = <int, int>{};
  late final int utf8Length;

  void requireByteBoundary(int offset, String field) {
    if (!_byteToUtf16.containsKey(offset)) {
      throw FormatException('$field is not a valid UTF-8 boundary.');
    }
  }

  int utf16ForByte(int offset) {
    return _byteToUtf16[offset] ??
        (throw const FormatException('Invalid UTF-8 byte offset.'));
  }

  int byteForUtf16(int offset) {
    return _utf16ToByte[offset] ??
        (throw const FormatException('Invalid UTF-16 code unit offset.'));
  }
}

int _utf8Length(int rune) {
  if (rune <= 0x7f) {
    return 1;
  }
  if (rune <= 0x7ff) {
    return 2;
  }
  if (rune <= 0xffff) {
    return 3;
  }
  return 4;
}

final class _LineRange {
  const _LineRange(this.start, this.end, this.hardBreak);

  final int start;
  final int end;
  final bool hardBreak;
}

final class _MeasureBatch {
  const _MeasureBatch(this.schemaVersion, this.requests);

  final int schemaVersion;
  final List<_MeasureRequest> requests;
}

final class _MeasureRequest {
  const _MeasureRequest({
    required this.id,
    required this.paragraphId,
    required this.text,
    required this.textStart,
    required this.textEnd,
    required this.styleRuns,
    required this.fontSize,
    required this.maxWidth,
    required this.availableWidth,
    required this.locale,
    required this.direction,
    required this.textScale,
    required this.fontCandidates,
    required this.strutAscent,
    required this.strutDescent,
    required this.strutLeading,
    required this.heightBehavior,
    required this.requestFingerprint,
  });

  final int id;
  final int paragraphId;
  final String text;
  final int textStart;
  final int textEnd;
  final List<_TextStyleRun> styleRuns;
  final int fontSize;
  final int maxWidth;
  final int availableWidth;
  final String locale;
  final _TextDirection direction;
  final int textScale;
  final _FontFallbackChain fontCandidates;
  final int strutAscent;
  final int strutDescent;
  final int strutLeading;
  final _HeightBehavior heightBehavior;
  final int requestFingerprint;
}

final class _TextStyleRun {
  const _TextStyleRun({
    required this.start,
    required this.end,
    required this.fontSize,
    required this.letterSpacing,
    required this.fonts,
  });

  final int start;
  final int end;
  final int fontSize;
  final int letterSpacing;
  final _FontFallbackChain fonts;
}

final class _FontFallbackChain {
  const _FontFallbackChain(this.primary, this.fallbacks);

  final _FontDescriptor primary;
  final List<_FontDescriptor> fallbacks;
}

final class _FontDescriptor {
  const _FontDescriptor({
    required this.family,
    required this.weight,
    required this.style,
    required this.stretch,
  });

  final String family;
  final int weight;
  final _FontStyle style;
  final int stretch;
}

enum _TextDirection { auto, ltr, rtl }

enum _FontStyle { normal, italic, oblique }

enum _HeightBehavior { natural, includeStrut, tight }

final class _MeasuredText {
  const _MeasuredText({
    required this.requestId,
    required this.requestFingerprint,
    required this.width,
    required this.height,
    required this.utf8Length,
    required this.lines,
    required this.clusters,
    required this.measurementFingerprint,
  });

  final int requestId;
  final int requestFingerprint;
  final int width;
  final int height;
  final int utf8Length;
  final List<_MeasuredLine> lines;
  final List<_MeasuredCluster> clusters;
  final int measurementFingerprint;
}

final class _MeasuredLine {
  const _MeasuredLine({
    required this.textStart,
    required this.textEnd,
    required this.baseline,
    required this.ascent,
    required this.descent,
    required this.lineHeight,
    required this.width,
    required this.inkX,
    required this.inkY,
    required this.inkWidth,
    required this.inkHeight,
    required this.hardBreak,
  });

  final int textStart;
  final int textEnd;
  final int baseline;
  final int ascent;
  final int descent;
  final int lineHeight;
  final int width;
  final int inkX;
  final int inkY;
  final int inkWidth;
  final int inkHeight;
  final bool hardBreak;
}

final class _MeasuredCluster {
  const _MeasuredCluster({
    required this.textStart,
    required this.textEnd,
    required this.lineIndex,
    required this.xStart,
    required this.xEnd,
  });

  final int textStart;
  final int textEnd;
  final int lineIndex;
  final int xStart;
  final int xEnd;
}

final class _MeasureWireCodec {
  const _MeasureWireCodec._();

  static _MeasureBatch decodeRequest(Uint8List bytes) {
    final envelope = _decodeEnvelope(bytes, _measurePayloadKind);
    final reader = _WireReader(envelope.payload);
    final count = reader.readCollectionLength('measure requests');
    final requests = <_MeasureRequest>[];
    final requestIds = <int>{};
    for (var index = 0; index < count; index += 1) {
      final request = _readRequest(reader, envelope.schemaVersion);
      if (!requestIds.add(request.id)) {
        throw const FormatException(
          'MeasureBatch contains duplicate request ids.',
        );
      }
      requests.add(request);
    }
    reader.finish();
    return _MeasureBatch(envelope.schemaVersion, requests);
  }

  static Uint8List encodeMeasured({
    required int schemaVersion,
    required int backendId,
    required int fontFingerprint,
    required List<_MeasuredText> results,
  }) {
    final writer = _WireWriter();
    writer
      ..writeUint64(backendId)
      ..writeUint64(fontFingerprint)
      ..writeCollectionLength('measured results', results.length);
    for (final result in results) {
      writer
        ..writeUint32(result.requestId)
        ..writeUint64(result.requestFingerprint)
        ..writeInt64(result.width)
        ..writeInt64(result.height)
        ..writeUint32(result.lines.length)
        ..writeUint32(result.utf8Length)
        ..writeCollectionLength('measured lines', result.lines.length);
      for (final line in result.lines) {
        writer
          ..writeUint32(line.textStart)
          ..writeUint32(line.textEnd)
          ..writeInt64(line.baseline)
          ..writeInt64(line.ascent)
          ..writeInt64(line.descent)
          ..writeInt64(line.lineHeight)
          ..writeInt64(line.width);
        if (schemaVersion != 1) {
          writer
            ..writeInt64(line.inkX)
            ..writeInt64(line.inkY)
            ..writeInt64(line.inkWidth)
            ..writeInt64(line.inkHeight);
        }
        writer.writeUint8(line.hardBreak ? 1 : 0);
      }
      writer.writeCollectionLength('measured clusters', result.clusters.length);
      for (final cluster in result.clusters) {
        writer
          ..writeUint32(cluster.textStart)
          ..writeUint32(cluster.textEnd)
          ..writeUint32(cluster.lineIndex)
          ..writeInt64(cluster.xStart)
          ..writeInt64(cluster.xEnd);
      }
      writer.writeUint64(result.measurementFingerprint);
    }
    return _encodeEnvelope(
      schemaVersion,
      _measuredPayloadKind,
      writer.takeBytes(),
    );
  }

  static _MeasureRequest _readRequest(_WireReader reader, int schemaVersion) {
    final id = reader.readUint32('measure request id');
    final paragraphId = reader.readUint32('paragraph id');
    final text = reader.readString('measure text');
    final textStart = reader.readUint32('measure text range start');
    final textEnd = reader.readUint32('measure text range end');
    final styleCount = reader.readCollectionLength('text style runs');
    final styleRuns = <_TextStyleRun>[];
    for (var index = 0; index < styleCount; index += 1) {
      styleRuns.add(
        _TextStyleRun(
          start: reader.readUint32('text style run start'),
          end: reader.readUint32('text style run end'),
          fontSize: reader.readInt64('text style font size'),
          letterSpacing: schemaVersion == 1
              ? 0
              : reader.readInt64('text style letter spacing'),
          fonts: _readFontChain(reader),
        ),
      );
    }
    final fontSize = reader.readInt64('font size');
    final maxWidth = reader.readInt64('maximum width');
    final availableWidth = reader.readInt64('available width');
    final locale = reader.readString('measure locale');
    final direction = _enumValue(
      _TextDirection.values,
      reader.readUint8('text direction'),
      'text direction',
    );
    final textScale = reader.readInt64('text scale');
    final fontCandidates = _readFontChain(reader);
    final strutAscent = reader.readInt64('strut ascent');
    final strutDescent = reader.readInt64('strut descent');
    final strutLeading = reader.readInt64('strut leading');
    final heightBehavior = _enumValue(
      _HeightBehavior.values,
      reader.readUint8('height behavior'),
      'height behavior',
    );
    final requestFingerprint = reader.readUint64('request fingerprint');
    final offsets = _TextOffsets(text);
    offsets.requireByteBoundary(textStart, 'measure text range start');
    offsets.requireByteBoundary(textEnd, 'measure text range end');
    if (textStart > textEnd) {
      throw const FormatException('Measure text range is reversed.');
    }
    var previousStyleEnd = textStart;
    for (final run in styleRuns) {
      offsets.requireByteBoundary(run.start, 'text style run start');
      offsets.requireByteBoundary(run.end, 'text style run end');
      if (run.start > run.end ||
          run.start < textStart ||
          run.end > textEnd ||
          run.start < previousStyleEnd) {
        throw const FormatException(
          'MeasureBatch contains overlapping or invalid style runs.',
        );
      }
      previousStyleEnd = run.end;
    }
    return _MeasureRequest(
      id: id,
      paragraphId: paragraphId,
      text: text,
      textStart: textStart,
      textEnd: textEnd,
      styleRuns: styleRuns,
      fontSize: fontSize,
      maxWidth: maxWidth,
      availableWidth: availableWidth,
      locale: locale,
      direction: direction,
      textScale: textScale,
      fontCandidates: fontCandidates,
      strutAscent: strutAscent,
      strutDescent: strutDescent,
      strutLeading: strutLeading,
      heightBehavior: heightBehavior,
      requestFingerprint: requestFingerprint,
    );
  }

  static _FontFallbackChain _readFontChain(_WireReader reader) {
    final primary = _readFont(reader);
    final count = reader.readCollectionLength('font fallbacks');
    final fallbacks = <_FontDescriptor>[];
    for (var index = 0; index < count; index += 1) {
      fallbacks.add(_readFont(reader));
    }
    return _FontFallbackChain(primary, fallbacks);
  }

  static _FontDescriptor _readFont(_WireReader reader) {
    final hasFontId = reader.readUint8('font id option');
    if (hasFontId > 1) {
      throw const FormatException('Wire option contains an invalid boolean.');
    }
    if (hasFontId == 1) {
      reader.readUint32('font id');
    }
    final family = reader.readString('font family');
    final weight = reader.readUint16('font weight');
    final style = _enumValue(
      _FontStyle.values,
      reader.readUint8('font style'),
      'font style',
    );
    final stretch = reader.readUint16('font stretch');
    reader.readUint64('font fingerprint');
    return _FontDescriptor(
      family: family,
      weight: weight,
      style: style,
      stretch: stretch,
    );
  }
}

T _enumValue<T>(List<T> values, int value, String field) {
  if (value >= values.length) {
    throw FormatException('$field contains an unknown value.');
  }
  return values[value];
}

final class _Envelope {
  const _Envelope(this.schemaVersion, this.payload);

  final int schemaVersion;
  final Uint8List payload;
}

_Envelope _decodeEnvelope(Uint8List bytes, int expectedKind) {
  if (bytes.length < _headerLength) {
    throw const FormatException('Wire payload is shorter than its header.');
  }
  for (var index = 0; index < _wireMagic.length; index += 1) {
    if (bytes[index] != _wireMagic[index]) {
      throw const FormatException('Wire payload has invalid magic.');
    }
  }
  final header = ByteData.sublistView(bytes, 0, _headerLength);
  final version = header.getUint16(8, Endian.little);
  if (version < 1 || version > _currentSchemaVersion) {
    throw FormatException('Unsupported wire schema version $version.');
  }
  final kind = header.getUint16(10, Endian.little);
  if (kind != expectedKind) {
    throw FormatException('Unexpected wire payload kind $kind.');
  }
  final declaredLength = header.getUint32(12, Endian.little);
  if (declaredLength > _maximumPayloadBytes) {
    throw const FormatException('Wire payload exceeds the allocation limit.');
  }
  final payload = Uint8List.sublistView(bytes, _headerLength);
  if (payload.length != declaredLength) {
    throw const FormatException(
        'Wire payload length does not match its header.');
  }
  final expectedCrc = header.getUint32(16, Endian.little);
  if (_crc32(payload) != expectedCrc) {
    throw const FormatException('Wire payload checksum does not match.');
  }
  return _Envelope(version, payload);
}

Uint8List _encodeEnvelope(int version, int kind, Uint8List payload) {
  if (version < 1 || version > _currentSchemaVersion) {
    throw ArgumentError.value(version, 'version', 'unsupported wire version');
  }
  if (payload.length > _maximumPayloadBytes) {
    throw const FormatException('Wire payload exceeds the allocation limit.');
  }
  final bytes = Uint8List(_headerLength + payload.length);
  bytes.setAll(0, _wireMagic);
  final header = ByteData.sublistView(bytes, 0, _headerLength);
  header
    ..setUint16(8, version, Endian.little)
    ..setUint16(10, kind, Endian.little)
    ..setUint32(12, payload.length, Endian.little)
    ..setUint32(16, _crc32(payload), Endian.little);
  bytes.setAll(_headerLength, payload);
  return bytes;
}

final class _WireReader {
  _WireReader(Uint8List bytes)
      : _bytes = bytes,
        _data = ByteData.sublistView(bytes);

  final Uint8List _bytes;
  final ByteData _data;
  int _offset = 0;

  void finish() {
    if (_offset != _bytes.length) {
      throw const FormatException('Wire payload contains trailing bytes.');
    }
  }

  int readUint8(String field) {
    _require(1, field);
    final value = _data.getUint8(_offset);
    _offset += 1;
    return value;
  }

  int readUint16(String field) {
    _require(2, field);
    final value = _data.getUint16(_offset, Endian.little);
    _offset += 2;
    return value;
  }

  int readUint32(String field) {
    _require(4, field);
    final value = _data.getUint32(_offset, Endian.little);
    _offset += 4;
    return value;
  }

  int readUint64(String field) {
    _require(8, field);
    final value = _data.getUint64(_offset, Endian.little);
    _offset += 8;
    return value;
  }

  int readInt64(String field) {
    _require(8, field);
    final value = _data.getInt64(_offset, Endian.little);
    _offset += 8;
    return value;
  }

  int readCollectionLength(String field) {
    final value = readUint32(field);
    if (value > _maximumCollectionItems) {
      throw FormatException('$field exceeds the collection limit.');
    }
    return value;
  }

  String readString(String field) {
    final length = readUint32(field);
    if (length > _maximumStringBytes) {
      throw FormatException('$field exceeds the string limit.');
    }
    _require(length, field);
    final value = utf8.decode(
      Uint8List.sublistView(_bytes, _offset, _offset + length),
      allowMalformed: false,
    );
    _offset += length;
    return value;
  }

  void _require(int length, String field) {
    if (length < 0 || _offset + length > _bytes.length) {
      throw FormatException('Wire payload ended while reading $field.');
    }
  }
}

final class _WireWriter {
  final BytesBuilder _bytes = BytesBuilder(copy: false);

  void writeUint8(int value) {
    _bytes.addByte(value);
  }

  void writeUint32(int value) {
    final data = ByteData(4)..setUint32(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void writeUint64(int value) {
    final data = ByteData(8)..setUint64(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void writeInt64(int value) {
    final data = ByteData(8)..setInt64(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void writeCollectionLength(String field, int value) {
    if (value > _maximumCollectionItems) {
      throw FormatException('$field exceeds the collection limit.');
    }
    writeUint32(value);
  }

  Uint8List takeBytes() => _bytes.takeBytes();
}

int _crc32(Uint8List bytes) {
  var crc = 0xffffffff;
  for (final byte in bytes) {
    crc ^= byte;
    for (var bit = 0; bit < 8; bit += 1) {
      crc = (crc & 1) == 1 ? (crc >> 1) ^ 0xedb88320 : crc >> 1;
    }
  }
  return (crc ^ 0xffffffff) & 0xffffffff;
}
