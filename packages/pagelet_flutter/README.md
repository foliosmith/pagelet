# pagelet_flutter

Flutter host bindings for the native `pagelet` EPUB parsing and pagination
engine.

The adapter owns the engine and book-session lifecycle and provides batched
Flutter paragraph measurement. Layout-session wrappers, page-scene decoding,
and resource loading are added by the following Milestone 4 tasks.

```dart
final engine = PageletEngine(libraryPath: '/path/to/libpagelet.dylib');
final book = engine.openBook('/path/to/book.epub');

try {
  final measurement = TextMeasureBridge(
    fontFingerprint: myRegisteredFontSetFingerprint,
  ).measureBatch(nativeMeasureBatchBytes);
  try {
    // Submit measurement.wireBytes once, then render with the retained
    // measurement.paragraphs TextPainter instances.
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
