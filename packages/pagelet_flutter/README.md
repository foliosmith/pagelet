# pagelet_flutter

Flutter host bindings for the native `pagelet` EPUB parsing and pagination
engine.

The adapter owns the engine and book-session lifecycle, provides batched
Flutter paragraph measurement, and decodes versioned page scenes into
Flutter-ready geometry. Layout-session wrappers and resource loading are added
by the following Milestone 4 tasks.

```dart
final engine = PageletEngine(libraryPath: '/path/to/libpagelet.dylib');
final book = engine.openBook('/path/to/book.epub');

try {
  final measurement = TextMeasureBridge(
    fontFingerprint: myRegisteredFontSetFingerprint,
  ).measureBatch(nativeMeasureBatchBytes);
  try {
    final page = const PageSceneDecoder()
        .decode(nativePageBatchBytes)
        .pages
        .first;
    // Paint page.textPaints with the matching retained TextPainters.
  } finally {
    measurement.dispose();
  }
} finally {
  book.dispose();
  engine.dispose();
}
```

If `libraryPath` is omitted, the package loads the platform library name:
`libpagelet.dylib`, `libpagelet.so`, or `pagelet.dll`. iOS resolves statically
linked symbols from the current process.
