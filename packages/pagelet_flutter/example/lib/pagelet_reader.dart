import 'package:flutter/material.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

final class PageletReader extends StatefulWidget {
  const PageletReader({
    required this.libraryPath,
    required this.bookPath,
    this.viewport = const Size(600, 800),
    super.key,
  });

  final String libraryPath;
  final String bookPath;
  final Size viewport;

  @override
  State<PageletReader> createState() => _PageletReaderState();
}

final class _PageletReaderState extends State<PageletReader> {
  _ReaderDocument? _document;
  Object? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
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
          viewport: widget.viewport,
        );
      });
      if (!mounted) {
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
              child: CustomPaint(
                size: page.size,
                painter: _PagePainter(page, document.paragraphs),
              ),
            ),
          ),
        );
      },
    );
  }
}

_ReaderDocument _loadDocument({
  required String libraryPath,
  required String bookPath,
  required Size viewport,
}) {
  final engine = PageletEngine(libraryPath: libraryPath);
  final measurements = <TextMeasurementBatch>[];
  try {
    final book = engine.openBook(bookPath);
    final chapter = book.openChapter(0);
    final layout = chapter.createLayout(
      PageletLayoutOptions(
        viewportWidth: viewport.width,
        viewportHeight: viewport.height,
        marginStart: 32,
        marginEnd: 32,
        marginTop: 40,
        marginBottom: 40,
        maxPages: 3,
      ),
    );
    var result = layout.requestPages(
      const PageletPageRequest(maxPages: 3),
    );
    while (result.state == PageletLayoutState.needMeasurements) {
      final measurement = const TextMeasureBridge(
        fontFingerprint: 0,
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
    required this.measurements,
    required this.pages,
    required this.paragraphs,
  });

  final PageletEngine engine;
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
  const _PagePainter(this.page, this.paragraphs);

  final PageScene page;
  final Map<int, MeasuredParagraph> paragraphs;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = Colors.white);
    for (final paint in page.textPaints) {
      final paragraph = paragraphs[paint.paragraphId];
      if (paragraph == null) {
        continue;
      }
      canvas
        ..save()
        ..clipRect(paint.clipRect);
      paragraph.painter.paint(canvas, paint.paintOrigin);
      canvas.restore();
    }
  }

  @override
  bool shouldRepaint(_PagePainter oldDelegate) {
    return oldDelegate.page != page || oldDelegate.paragraphs != paragraphs;
  }
}
