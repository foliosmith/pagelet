import 'dart:async';

import 'package:flutter/material.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

import 'reader_configuration.dart';
import 'reader_interactions.dart';

final class PageletReader extends StatefulWidget {
  const PageletReader({
    required this.libraryPath,
    required this.bookPath,
    this.configuration = const PageletReaderConfiguration(),
    this.onLinkTap,
    this.onImageTap,
    super.key,
  });

  final String libraryPath;
  final String bookPath;
  final PageletReaderConfiguration configuration;
  final ValueChanged<PageLinkRegion>? onLinkTap;
  final ValueChanged<SceneFragment>? onImageTap;

  @override
  State<PageletReader> createState() => _PageletReaderState();
}

final class _PageletReaderState extends State<PageletReader> {
  _ReaderDocument? _document;
  Object? _error;
  int _generation = 0;
  int? _selectionPageIndex;
  List<Rect> _selectionRects = const <Rect>[];

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(PageletReader oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.libraryPath != widget.libraryPath ||
        oldWidget.bookPath != widget.bookPath ||
        widget.configuration.impactFrom(oldWidget.configuration) !=
            ReaderLayoutImpact.paintOnly) {
      _load();
    }
  }

  Future<void> _load() async {
    final generation = ++_generation;
    final previous = _document;
    setState(() {
      _document = null;
      _error = null;
    });
    previous?.dispose();
    try {
      final document = await Future<_ReaderDocument>(() {
        return _loadDocument(
          libraryPath: widget.libraryPath,
          bookPath: widget.bookPath,
          configuration: widget.configuration,
        );
      });
      if (!mounted || generation != _generation) {
        document.dispose();
        return;
      }
      setState(() => _document = document);
    } catch (error) {
      if (mounted) {
        setState(() => _error = error);
      }
    }
  }

  @override
  void dispose() {
    _generation += 1;
    _document?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final error = _error;
    if (error != null) {
      return Center(child: SelectableText(error.toString()));
    }
    final document = _document;
    if (document == null) {
      return const Center(child: CircularProgressIndicator());
    }
    return PageView.builder(
      itemCount: document.pages.length,
      itemBuilder: (context, index) {
        final page = document.pages[index];
        return Center(
          child: FittedBox(
            fit: BoxFit.contain,
            child: RepaintBoundary(
              key: ValueKey<String>('page-${page.pageIndex}'),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTapUp: (details) => unawaited(
                  _handleTap(page, details.localPosition),
                ),
                child: CustomPaint(
                  size: page.size,
                  painter: _PagePainter(
                    page,
                    document.paragraphs,
                    widget.configuration,
                    _selectionPageIndex == page.pageIndex
                        ? _selectionRects
                        : const <Rect>[],
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  Future<void> _handleTap(PageScene page, Offset position) async {
    final target = resolveReaderTap(page, position);
    if (target case ReaderLinkTap(:final link)) {
      final callback = widget.onLinkTap;
      if (callback != null) {
        callback(link);
      } else if (link.kind == PageLinkKind.footnote) {
        await showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            title: const Text('Footnote'),
            content: SelectableText(link.href),
          ),
        );
      } else if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text(link.href)),
        );
      }
      return;
    }
    if (target case ReaderImageTap(:final fragment)) {
      final callback = widget.onImageTap;
      if (callback != null) {
        callback(fragment);
      } else {
        // ponytail: PageScene has no image resource id yet; replace this
        // placeholder with book.resources.read(id) when the wire exposes it.
        await showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            content: InteractiveViewer(
              child: SizedBox.fromSize(
                size: fragment.rect.size,
                child: ColoredBox(
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                  child: Center(
                    child: Text(fragment.text ?? 'Image'),
                  ),
                ),
              ),
            ),
          ),
        );
      }
      return;
    }
    final document = _document;
    if (document == null) {
      return;
    }
    final hit = document.layout.hitTest(page.pageIndex, position);
    if (!mounted) {
      return;
    }
    setState(() {
      _selectionPageIndex = page.pageIndex;
      _selectionRects =
          hit == null ? const <Rect>[] : _selectionRectsForHit(page, hit);
    });
  }
}

_ReaderDocument _loadDocument({
  required String libraryPath,
  required String bookPath,
  required PageletReaderConfiguration configuration,
}) {
  final engine = PageletEngine(libraryPath: libraryPath);
  final measurements = <TextMeasurementBatch>[];
  try {
    final book = engine.openBook(bookPath);
    final chapter = book.openChapter(0);
    final layout = chapter.createLayout(
      PageletLayoutOptions(
        viewportWidth: configuration.viewportWidth,
        viewportHeight: configuration.viewportHeight,
        marginStart: configuration.marginStart,
        marginEnd: configuration.marginEnd,
        marginTop: configuration.marginTop,
        marginBottom: configuration.marginBottom,
        maxPages: 3,
      ),
    );
    var result = layout.requestPages(
      const PageletPageRequest(maxPages: 3),
    );
    while (result.state == PageletLayoutState.needMeasurements) {
      final measurement = TextMeasureBridge(
        fontFingerprint: configuration.fontFingerprint,
      ).measureBatch(result.bytes);
      measurements.add(measurement);
      result = layout.submitMeasurements(
        result.requestHandle,
        measurement.wireBytes,
      );
    }
    if (result.state != PageletLayoutState.pages) {
      throw const FormatException('The first chapter produced no pages.');
    }
    final pages = const PageSceneDecoder().decode(result.bytes).pages;
    if (pages.isEmpty) {
      throw const FormatException('The first page batch is empty.');
    }
    final paragraphs = <int, MeasuredParagraph>{};
    for (final measurement in measurements) {
      paragraphs.addAll(measurement.paragraphs);
    }
    for (final page in pages) {
      for (final paragraph in page.paragraphs) {
        final measured = paragraphs[paragraph.paragraphId];
        if (measured == null ||
            measured.requestFingerprint != paragraph.requestFingerprint ||
            measured.measurementFingerprint !=
                paragraph.measurementFingerprint) {
          throw const FormatException(
            'Page scene paragraph does not match retained measurement.',
          );
        }
      }
    }
    return _ReaderDocument(
      engine: engine,
      layout: layout,
      measurements: measurements,
      pages: pages,
      paragraphs: paragraphs,
    );
  } catch (_) {
    for (final measurement in measurements) {
      measurement.dispose();
    }
    engine.dispose();
    rethrow;
  }
}

final class _ReaderDocument {
  const _ReaderDocument({
    required this.engine,
    required this.layout,
    required this.measurements,
    required this.pages,
    required this.paragraphs,
  });

  final PageletEngine engine;
  final LayoutSession layout;
  final List<TextMeasurementBatch> measurements;
  final List<PageScene> pages;
  final Map<int, MeasuredParagraph> paragraphs;

  void dispose() {
    for (final measurement in measurements) {
      measurement.dispose();
    }
    engine.dispose();
  }
}

final class _PagePainter extends CustomPainter {
  const _PagePainter(
    this.page,
    this.paragraphs,
    this.configuration,
    this.selectionRects,
  );

  final PageScene page;
  final Map<int, MeasuredParagraph> paragraphs;
  final PageletReaderConfiguration configuration;
  final List<Rect> selectionRects;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(
      Offset.zero & size,
      Paint()..color = configuration.backgroundColor,
    );
    for (final rect in selectionRects) {
      canvas.drawRect(
        rect,
        Paint()..color = Colors.amber.withValues(alpha: 0.35),
      );
    }
    for (final paint in page.textPaints) {
      final paragraph = paragraphs[paint.paragraphId];
      if (paragraph == null) {
        continue;
      }
      canvas
        ..save()
        ..clipRect(paint.clipRect)
        ..saveLayer(
          paint.clipRect,
          Paint()
            ..colorFilter = ColorFilter.mode(
              configuration.textColor,
              BlendMode.srcIn,
            ),
        );
      paragraph.painter.paint(canvas, paint.paintOrigin);
      canvas
        ..restore()
        ..restore();
    }
  }

  @override
  bool shouldRepaint(_PagePainter oldDelegate) {
    return oldDelegate.page != page ||
        oldDelegate.paragraphs != paragraphs ||
        oldDelegate.configuration != configuration ||
        oldDelegate.selectionRects != selectionRects;
  }
}

List<Rect> _selectionRectsForHit(
  PageScene page,
  PageletHitTestResult hit,
) {
  for (final selection in page.selections) {
    if (selection.nodeId == hit.nodeId &&
        selection.range.start <= hit.utf8ByteOffset &&
        hit.utf8ByteOffset < selection.range.end) {
      return selection.rects;
    }
  }
  for (final paint in page.textPaints) {
    if (paint.id != hit.fragmentId) {
      continue;
    }
    SceneParagraph? paragraph;
    for (final value in page.paragraphs) {
      if (value.paragraphId == paint.paragraphId) {
        paragraph = value;
        break;
      }
    }
    if (paragraph == null) {
      return const <Rect>[];
    }
    for (final cluster in paragraph.clusters) {
      if (cluster.textRange.start <= hit.utf8ByteOffset &&
          hit.utf8ByteOffset < cluster.textRange.end &&
          cluster.lineIndex < paragraph.lines.length) {
        final line = paragraph.lines[cluster.lineIndex];
        return <Rect>[
          Rect.fromLTWH(
            paint.paintOrigin.dx + cluster.xStart,
            paint.paintOrigin.dy + line.layoutRect.top,
            cluster.xEnd - cluster.xStart,
            line.layoutRect.height,
          ),
        ];
      }
    }
  }
  return const <Rect>[];
}
