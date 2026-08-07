# pagelet_flutter

Flutter host bindings for the native `pagelet` EPUB parsing and pagination
engine.

The first adapter slice owns the engine and book-session lifecycle. Layout,
host text measurement, page-scene decoding, and resource loading are added by
the following Milestone 4 tasks.

```dart
final engine = PageletEngine(libraryPath: '/path/to/libpagelet.dylib');
final book = engine.openBook('/path/to/book.epub');

try {
  // Create chapter and layout sessions in the subsequent adapter slices.
} finally {
  book.dispose();
  engine.dispose();
}
```

If `libraryPath` is omitted, the package loads the platform library name:
`libpagelet.dylib`, `libpagelet.so`, or `pagelet.dll`. iOS resolves statically
linked symbols from the current process.
