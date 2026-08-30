import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui';

const int _layoutUnitScale = 64;
const int _currentSchemaVersion = 3;
const int _pagePayloadKind = 1;
const int _headerLength = 20;
const int _maximumPayloadBytes = 64 * 1024 * 1024;
const int _maximumCollectionItems = 1000000;
const int _maximumStringBytes = 8 * 1024 * 1024;
const List<int> _wireMagic = <int>[
  0x50,
  0x47,
  0x4c,
  0x54,
  0x53,
  0x43,
  0x4e,
  0,
];

/// Decodes a versioned pagelet `PageBatch` into Flutter-ready page scenes.
final class PageSceneDecoder {
  const PageSceneDecoder();

  PageSceneBatch decode(Uint8List bytes) {
    final envelope = _decodeEnvelope(bytes);
    final reader = _Reader(envelope.payload);
    final pages = <PageScene>[];
    final count = reader.collectionLength('pages');
    for (var index = 0; index < count; index += 1) {
      pages.add(
        envelope.schemaVersion == 1
            ? _readLegacyPage(reader)
            : _readModernPage(reader, envelope.schemaVersion),
      );
    }
    reader.finish();
    return PageSceneBatch(
      schemaVersion: envelope.schemaVersion,
      pages: List<PageScene>.unmodifiable(pages),
    );
  }
}

final class PageSceneBatch {
  const PageSceneBatch({required this.schemaVersion, required this.pages});

  final int schemaVersion;
  final List<PageScene> pages;
}

final class PageScene {
  const PageScene({
    required this.pageIndex,
    required this.size,
    required this.startAnchor,
    required this.endAnchor,
    required this.textBackendId,
    required this.fontFingerprint,
    required this.paragraphs,
    required this.textPaints,
    required this.fragments,
    required this.links,
    required this.anchors,
    required this.selections,
    required this.semantics,
    required this.fingerprint,
    required this.nextBreakToken,
    required this.diagnostics,
  });

  final int pageIndex;
  final Size size;
  final PageTextAnchor? startAnchor;
  final PageTextAnchor? endAnchor;
  final int textBackendId;
  final int fontFingerprint;
  final List<SceneParagraph> paragraphs;
  final List<TextPaintFragment> textPaints;
  final List<SceneFragment> fragments;
  final List<PageLinkRegion> links;
  final List<PageAnchorRegion> anchors;
  final List<PageSelectionMap> selections;
  final List<PageSemanticNode> semantics;
  final String fingerprint;
  final PageBreakToken? nextBreakToken;
  final List<PageDiagnostic> diagnostics;
}

final class SceneParagraph {
  const SceneParagraph({
    required this.paragraphId,
    required this.requestFingerprint,
    required this.measurementFingerprint,
    required this.text,
    required this.textRange,
    required this.lines,
    required this.clusters,
  });

  final int paragraphId;
  final int requestFingerprint;
  final int measurementFingerprint;
  final String text;
  final PageSourceRange textRange;
  final List<PageSceneLine> lines;
  final List<PageTextCluster> clusters;
}

final class PageSceneLine {
  const PageSceneLine({
    required this.textRange,
    required this.baseline,
    required this.ascent,
    required this.descent,
    required this.height,
    required this.width,
    required this.metricsInkBounds,
    required this.inkBounds,
    required this.layoutRect,
    required this.hardBreak,
  });

  final PageSourceRange textRange;
  final double baseline;
  final double ascent;
  final double descent;
  final double height;
  final double width;
  final Rect metricsInkBounds;
  final Rect inkBounds;
  final Rect layoutRect;
  final bool hardBreak;
}

final class PageTextCluster {
  const PageTextCluster({
    required this.textRange,
    required this.lineIndex,
    required this.xStart,
    required this.xEnd,
  });

  final PageSourceRange textRange;
  final int lineIndex;
  final double xStart;
  final double xEnd;
}

final class TextPaintFragment {
  const TextPaintFragment({
    required this.id,
    required this.nodeId,
    required this.paragraphId,
    required this.visibleTextRange,
    required this.paintOrigin,
    required this.layoutRect,
    required this.clipRect,
    required this.firstLine,
    required this.lineCount,
    required this.sourceRange,
    required this.anchorRange,
    required this.overflow,
  });

  final int id;
  final int nodeId;
  final int paragraphId;
  final PageSourceRange visibleTextRange;
  final Offset paintOrigin;
  final Rect layoutRect;
  final Rect clipRect;
  final int firstLine;
  final int lineCount;
  final PageSourceRange? sourceRange;
  final PageTextAnchorRange anchorRange;
  final bool overflow;

  double get clipTop => clipRect.top;
  double get clipHeight => clipRect.height;
}

enum SceneFragmentKind {
  textLine,
  marker,
  image,
  divider,
  backgroundBorder,
  debugOverlay,
  unsupportedPlaceholder,
}

final class SceneFragment {
  const SceneFragment({
    required this.id,
    required this.kind,
    required this.nodeId,
    required this.rect,
    required this.text,
    required this.sourceRange,
    required this.anchorRange,
    required this.lineIndex,
    required this.overflow,
  });

  final int id;
  final SceneFragmentKind kind;
  final int nodeId;
  final Rect rect;
  final String? text;
  final PageSourceRange? sourceRange;
  final PageTextAnchorRange? anchorRange;
  final int? lineIndex;
  final bool overflow;
}

enum PageLinkKind { internal, external, resource, footnote, unknown }

final class PageLinkRegion {
  const PageLinkRegion({
    required this.rect,
    required this.nodeId,
    required this.textRange,
    required this.href,
    required this.resolvedDocument,
    required this.fragment,
    required this.kind,
  });

  final Rect rect;
  final int nodeId;
  final PageSourceRange? textRange;
  final String href;
  final String? resolvedDocument;
  final String? fragment;
  final PageLinkKind kind;
}

final class PageAnchorRegion {
  const PageAnchorRegion({
    required this.rect,
    required this.key,
    required this.nodeId,
  });

  final Rect rect;
  final String key;
  final int nodeId;
}

final class PageSelectionMap {
  const PageSelectionMap({
    required this.nodeId,
    required this.range,
    required this.rects,
  });

  final int nodeId;
  final PageSourceRange range;
  final List<Rect> rects;
}

final class PageSemanticNode {
  const PageSemanticNode({
    required this.nodeId,
    required this.rect,
    required this.role,
    required this.label,
  });

  final int nodeId;
  final Rect rect;
  final String role;
  final String label;
}

final class PageSourceRange {
  const PageSourceRange(this.start, this.end);

  final int start;
  final int end;
}

enum PageTextAffinity { upstream, downstream }

final class PageTextAnchor {
  const PageTextAnchor({
    required this.documentId,
    required this.nodeId,
    required this.utf8ByteOffset,
    required this.affinity,
  });

  final int documentId;
  final int nodeId;
  final int utf8ByteOffset;
  final PageTextAffinity affinity;
}

final class PageTextAnchorRange {
  const PageTextAnchorRange({required this.start, required this.end});

  final PageTextAnchor start;
  final PageTextAnchor end;
}

final class PageBreakToken {
  const PageBreakToken({
    required this.nodeId,
    required this.childIndex,
    required this.textOffset,
    required this.continuation,
    required this.pageIndex,
    required this.contentFingerprint,
    required this.configFingerprint,
    required this.textBackendId,
    required this.fontFingerprint,
  });

  final int nodeId;
  final int childIndex;
  final int textOffset;
  final bool continuation;
  final int pageIndex;
  final String contentFingerprint;
  final int configFingerprint;
  final int textBackendId;
  final int fontFingerprint;
}

enum PageDiagnosticCode {
  io,
  invalidContainer,
  invalidPackage,
  unsupportedFeature,
  resourceLimitExceeded,
  parse,
  layout,
  cancelled,
  protocol,
  internal,
}

enum PageDiagnosticSeverity { info, warning, error }

final class PageDiagnostic {
  const PageDiagnostic({
    required this.code,
    required this.severity,
    required this.message,
    required this.resourceId,
    required this.sourceRange,
  });

  final PageDiagnosticCode code;
  final PageDiagnosticSeverity severity;
  final String message;
  final int? resourceId;
  final PageSourceRange? sourceRange;
}

PageScene _readModernPage(_Reader reader, int version) {
  final pageIndex = reader.uint32('page index');
  final size = reader.size();
  final startAnchor = reader.option('start anchor', reader.textAnchor);
  final endAnchor = reader.option('end anchor', reader.textAnchor);
  final textBackendId = reader.uint64('text backend id');
  final fontFingerprint = reader.uint64('font fingerprint');
  final paragraphs = reader.list('paragraphs', () => _readParagraph(reader));
  final paragraphById = <int, SceneParagraph>{};
  for (final paragraph in paragraphs) {
    if (paragraphById[paragraph.paragraphId] != null) {
      throw const FormatException('Page contains duplicate paragraph ids.');
    }
    paragraphById[paragraph.paragraphId] = paragraph;
  }
  final textPaints = reader.list(
    'text paints',
    () => _readTextPaint(reader, paragraphById),
  );
  return PageScene(
    pageIndex: pageIndex,
    size: size,
    startAnchor: startAnchor,
    endAnchor: endAnchor,
    textBackendId: textBackendId,
    fontFingerprint: fontFingerprint,
    paragraphs: List<SceneParagraph>.unmodifiable(paragraphs),
    textPaints: List<TextPaintFragment>.unmodifiable(textPaints),
    fragments: List<SceneFragment>.unmodifiable(
      reader.list('fragments', () => _readFragment(reader)),
    ),
    links: List<PageLinkRegion>.unmodifiable(
      reader.list('links', () => _readLink(reader, version)),
    ),
    anchors: List<PageAnchorRegion>.unmodifiable(
      reader.list('anchors', () => _readAnchor(reader)),
    ),
    selections: List<PageSelectionMap>.unmodifiable(
      reader.list('selections', () => _readSelection(reader)),
    ),
    semantics: List<PageSemanticNode>.unmodifiable(
      reader.list('semantics', () => _readSemantic(reader)),
    ),
    fingerprint: reader.hash('page fingerprint'),
    nextBreakToken: reader.option('next break token', reader.breakToken),
    diagnostics: List<PageDiagnostic>.unmodifiable(
      reader.list('diagnostics', () => _readDiagnostic(reader)),
    ),
  );
}

PageScene _readLegacyPage(_Reader reader) {
  final pageIndex = reader.uint32('page index');
  final size = reader.size();
  final startAnchor = reader.option('start anchor', reader.textAnchor);
  final endAnchor = reader.option('end anchor', reader.textAnchor);
  return PageScene(
    pageIndex: pageIndex,
    size: size,
    startAnchor: startAnchor,
    endAnchor: endAnchor,
    textBackendId: 0,
    fontFingerprint: 0,
    paragraphs: const <SceneParagraph>[],
    textPaints: const <TextPaintFragment>[],
    fragments: List<SceneFragment>.unmodifiable(
      reader.list('fragments', () => _readFragment(reader)),
    ),
    links: List<PageLinkRegion>.unmodifiable(
      reader.list('links', () => _readLink(reader, 1)),
    ),
    anchors: List<PageAnchorRegion>.unmodifiable(
      reader.list('anchors', () => _readAnchor(reader)),
    ),
    selections: List<PageSelectionMap>.unmodifiable(
      reader.list('selections', () => _readSelection(reader)),
    ),
    semantics: List<PageSemanticNode>.unmodifiable(
      reader.list('semantics', () => _readSemantic(reader)),
    ),
    fingerprint: reader.hash('page fingerprint'),
    nextBreakToken: reader.option('next break token', reader.breakToken),
    diagnostics: List<PageDiagnostic>.unmodifiable(
      reader.list('diagnostics', () => _readDiagnostic(reader)),
    ),
  );
}

SceneParagraph _readParagraph(_Reader reader) {
  final paragraphId = reader.uint32('paragraph id');
  final requestFingerprint = reader.uint64('request fingerprint');
  final measurementFingerprint = reader.uint64('measurement fingerprint');
  final text = reader.string('paragraph text');
  final textRange = reader.range('paragraph text range');
  _requireTextRange(text, textRange, 'paragraph text range');
  final styleCount = reader.collectionLength('style runs');
  for (var index = 0; index < styleCount; index += 1) {
    final range = reader.range('style run range');
    _requireTextRange(text, range, 'style run range');
    if (range.start < textRange.start || range.end > textRange.end) {
      throw const FormatException('Style run is outside the paragraph range.');
    }
    reader
      ..layoutUnit('style font size')
      ..layoutUnit('style letter spacing')
      ..fontChain();
  }
  reader
    ..layoutUnit('paragraph font size')
    ..layoutUnit('paragraph available width')
    ..layoutUnit('paragraph maximum width')
    ..string('paragraph locale')
    ..enumValue('paragraph direction', 3)
    ..layoutUnit('paragraph text scale')
    ..fontChain()
    ..layoutUnit('strut ascent')
    ..layoutUnit('strut descent')
    ..layoutUnit('strut leading')
    ..enumValue('height behavior', 3);
  final lines = reader.list('paragraph lines', () => _readLine(reader));
  final clusters = reader.list('paragraph clusters', () {
    final range = reader.range('cluster range');
    _requireTextRange(text, range, 'cluster range');
    final lineIndex = reader.uint32('cluster line index');
    if (lineIndex >= lines.length && lines.isNotEmpty) {
      throw const FormatException('Cluster references an unknown line.');
    }
    return PageTextCluster(
      textRange: range,
      lineIndex: lineIndex,
      xStart: reader.layoutUnit('cluster x start'),
      xEnd: reader.layoutUnit('cluster x end'),
    );
  });
  return SceneParagraph(
    paragraphId: paragraphId,
    requestFingerprint: requestFingerprint,
    measurementFingerprint: measurementFingerprint,
    text: text,
    textRange: textRange,
    lines: List<PageSceneLine>.unmodifiable(lines),
    clusters: List<PageTextCluster>.unmodifiable(clusters),
  );
}

PageSceneLine _readLine(_Reader reader) {
  final textRange = reader.range('line text range');
  final baseline = reader.layoutUnit('line baseline');
  final ascent = reader.layoutUnit('line ascent');
  final descent = reader.layoutUnit('line descent');
  final height = reader.layoutUnit('line height');
  final width = reader.layoutUnit('line width');
  final metricsInk = reader.rect();
  final hardBreak = reader.boolean('line hard break');
  final layoutRect = reader.rect();
  final retainedInk = reader.rect();
  return PageSceneLine(
    textRange: textRange,
    baseline: baseline,
    ascent: ascent,
    descent: descent,
    height: height,
    width: width,
    metricsInkBounds: metricsInk,
    inkBounds: retainedInk,
    layoutRect: layoutRect,
    hardBreak: hardBreak,
  );
}

TextPaintFragment _readTextPaint(
  _Reader reader,
  Map<int, SceneParagraph> paragraphById,
) {
  final id = reader.uint32('text paint id');
  final nodeId = reader.uint32('text paint node id');
  final paragraphId = reader.uint32('text paint paragraph id');
  final paragraph = paragraphById[paragraphId];
  if (paragraph == null) {
    throw const FormatException('Text paint references an unknown paragraph.');
  }
  final visibleRange = reader.range('text paint visible range');
  _requireTextRange(paragraph.text, visibleRange, 'text paint visible range');
  final paintOrigin = reader.offset();
  final layoutRect = reader.rect();
  final clipRect = reader.rect();
  final firstLine = reader.uint32('text paint first line');
  final lineCount = reader.uint32('text paint line count');
  if (firstLine + lineCount > paragraph.lines.length) {
    throw const FormatException('Text paint references unknown lines.');
  }
  return TextPaintFragment(
    id: id,
    nodeId: nodeId,
    paragraphId: paragraphId,
    visibleTextRange: visibleRange,
    paintOrigin: paintOrigin,
    layoutRect: layoutRect,
    clipRect: clipRect,
    firstLine: firstLine,
    lineCount: lineCount,
    sourceRange: reader.option('text paint source range', reader.sourceRange),
    anchorRange: reader.textAnchorRange(),
    overflow: reader.boolean('text paint overflow'),
  );
}

SceneFragment _readFragment(_Reader reader) {
  return SceneFragment(
    id: reader.uint32('fragment id'),
    kind: SceneFragmentKind.values[
        reader.enumValue('fragment kind', SceneFragmentKind.values.length)],
    nodeId: reader.uint32('fragment node id'),
    rect: reader.rect(),
    text: reader.option('fragment text', () => reader.string('fragment text')),
    sourceRange: reader.option('fragment source range', reader.sourceRange),
    anchorRange: reader.option(
      'fragment anchor range',
      reader.textAnchorRange,
    ),
    lineIndex: reader.option(
      'fragment line index',
      () => reader.uint32('fragment line index'),
    ),
    overflow: reader.boolean('fragment overflow'),
  );
}

PageLinkRegion _readLink(_Reader reader, int version) {
  final rect = reader.rect();
  final nodeId = reader.uint32('link node id');
  final href = reader.string('link href');
  final resolvedDocument = reader.option(
    'resolved link document',
    () => reader.string('resolved link document'),
  );
  final fragment = reader.option(
    'link fragment',
    () => reader.string('link fragment'),
  );
  final kind = PageLinkKind
      .values[reader.enumValue('link kind', PageLinkKind.values.length)];
  return PageLinkRegion(
    rect: rect,
    nodeId: nodeId,
    textRange: version == 3
        ? reader.option('link text range', reader.sourceRange)
        : null,
    href: href,
    resolvedDocument: resolvedDocument,
    fragment: fragment,
    kind: kind,
  );
}

PageAnchorRegion _readAnchor(_Reader reader) => PageAnchorRegion(
      rect: reader.rect(),
      key: reader.string('anchor key'),
      nodeId: reader.uint32('anchor node id'),
    );

PageSelectionMap _readSelection(_Reader reader) => PageSelectionMap(
      nodeId: reader.uint32('selection node id'),
      range: reader.range('selection range'),
      rects: List<Rect>.unmodifiable(
        reader.list('selection rectangles', reader.rect),
      ),
    );

PageSemanticNode _readSemantic(_Reader reader) => PageSemanticNode(
      nodeId: reader.uint32('semantic node id'),
      rect: reader.rect(),
      role: reader.string('semantic role'),
      label: reader.string('semantic label'),
    );

PageDiagnostic _readDiagnostic(_Reader reader) => PageDiagnostic(
      code: PageDiagnosticCode.values[reader.enumValue(
          'diagnostic code', PageDiagnosticCode.values.length)],
      severity: PageDiagnosticSeverity.values[reader.enumValue(
        'diagnostic severity',
        PageDiagnosticSeverity.values.length,
      )],
      message: reader.string('diagnostic message'),
      resourceId: reader.option(
        'diagnostic resource',
        () => reader.uint32('diagnostic resource'),
      ),
      sourceRange: reader.option(
        'diagnostic source range',
        reader.sourceRange,
      ),
    );

void _requireTextRange(String text, PageSourceRange range, String field) {
  final bytes = utf8.encode(text);
  if (range.end > bytes.length ||
      (range.start < bytes.length && (bytes[range.start] & 0xc0) == 0x80) ||
      (range.end < bytes.length && (bytes[range.end] & 0xc0) == 0x80)) {
    throw FormatException('$field is not on UTF-8 boundaries.');
  }
}

final class _Envelope {
  const _Envelope(this.schemaVersion, this.payload);

  final int schemaVersion;
  final Uint8List payload;
}

_Envelope _decodeEnvelope(Uint8List bytes) {
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
  if (kind != _pagePayloadKind) {
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
  if (_crc32(payload) != header.getUint32(16, Endian.little)) {
    throw const FormatException('Wire payload checksum does not match.');
  }
  return _Envelope(version, payload);
}

final class _Reader {
  _Reader(Uint8List bytes)
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

  int uint8(String field) {
    _require(1, field);
    final value = _data.getUint8(_offset);
    _offset += 1;
    return value;
  }

  int uint16(String field) {
    _require(2, field);
    final value = _data.getUint16(_offset, Endian.little);
    _offset += 2;
    return value;
  }

  int uint32(String field) {
    _require(4, field);
    final value = _data.getUint32(_offset, Endian.little);
    _offset += 4;
    return value;
  }

  int uint64(String field) {
    _require(8, field);
    final value = _data.getUint64(_offset, Endian.little);
    _offset += 8;
    return value;
  }

  int int64(String field) {
    _require(8, field);
    final value = _data.getInt64(_offset, Endian.little);
    _offset += 8;
    return value;
  }

  double layoutUnit(String field) => int64(field) / _layoutUnitScale;

  bool boolean(String field) {
    final value = uint8(field);
    if (value > 1) {
      throw FormatException('$field has invalid boolean $value.');
    }
    return value == 1;
  }

  int enumValue(String field, int count) {
    final value = uint8(field);
    if (value >= count) {
      throw FormatException('$field has unknown value $value.');
    }
    return value;
  }

  int collectionLength(String field) {
    final value = uint32(field);
    if (value > _maximumCollectionItems) {
      throw FormatException('$field exceeds the collection limit.');
    }
    return value;
  }

  List<T> list<T>(String field, T Function() readValue) {
    final values = <T>[];
    final count = collectionLength(field);
    for (var index = 0; index < count; index += 1) {
      values.add(readValue());
    }
    return values;
  }

  T? option<T>(String field, T Function() readValue) {
    return boolean(field) ? readValue() : null;
  }

  String string(String field) {
    final length = uint32(field);
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

  String hash(String field) {
    _require(32, field);
    final buffer = StringBuffer();
    for (var index = 0; index < 32; index += 1) {
      buffer.write(_bytes[_offset + index].toRadixString(16).padLeft(2, '0'));
    }
    _offset += 32;
    return buffer.toString();
  }

  Size size() => Size(
        layoutUnit('page width'),
        layoutUnit('page height'),
      );

  Rect rect() => Rect.fromLTWH(
        layoutUnit('rectangle x'),
        layoutUnit('rectangle y'),
        layoutUnit('rectangle width'),
        layoutUnit('rectangle height'),
      );

  Offset offset() => Offset(
        layoutUnit('point x'),
        layoutUnit('point y'),
      );

  PageSourceRange range(String field) {
    final range = PageSourceRange(uint32('$field start'), uint32('$field end'));
    if (range.start > range.end) {
      throw FormatException('$field is reversed.');
    }
    return range;
  }

  PageSourceRange sourceRange() => range('source range');

  PageTextAnchor textAnchor() => PageTextAnchor(
        documentId: uint32('anchor document id'),
        nodeId: uint32('anchor node id'),
        utf8ByteOffset: uint32('anchor UTF-8 offset'),
        affinity: PageTextAffinity.values[
            enumValue('anchor affinity', PageTextAffinity.values.length)],
      );

  PageTextAnchorRange textAnchorRange() => PageTextAnchorRange(
        start: textAnchor(),
        end: textAnchor(),
      );

  PageBreakToken breakToken() => PageBreakToken(
        nodeId: uint32('break token node id'),
        childIndex: uint32('break token child index'),
        textOffset: uint32('break token text offset'),
        continuation: boolean('break token continuation'),
        pageIndex: uint32('break token page index'),
        contentFingerprint: hash('break token content fingerprint'),
        configFingerprint: uint64('break token config fingerprint'),
        textBackendId: uint64('break token text backend id'),
        fontFingerprint: uint64('break token font fingerprint'),
      );

  void fontChain() {
    fontDescriptor();
    final count = collectionLength('font fallbacks');
    for (var index = 0; index < count; index += 1) {
      fontDescriptor();
    }
  }

  void fontDescriptor() {
    option('font id', () => uint32('font id'));
    string('font family');
    uint16('font weight');
    enumValue('font style', 3);
    uint16('font stretch');
    uint64('font fingerprint');
  }

  void _require(int length, String field) {
    if (length < 0 || _offset + length > _bytes.length) {
      throw FormatException('Wire payload ended while reading $field.');
    }
  }
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
