part of 'engine.dart';

/// Owns one native EPUB book handle under a [PageletEngine].
final class BookSession {
  BookSession._(this._engine, this._handle);

  final PageletEngine _engine;
  final int _handle;
  final Set<ChapterSession> _chapters = <ChapterSession>{};
  bool _isDisposed = false;

  /// Lazily reads publication resources owned by this book.
  late final PageletResourceLoader resources = PageletResourceLoader._(this);

  /// Opens one zero-based spine item for layout.
  ChapterSession openChapter(int spineIndex) {
    _ensureOpen();
    if (spineIndex < 0) {
      throw RangeError.value(spineIndex, 'spineIndex', 'must be non-negative');
    }
    final handle = _requireHandle(
      _engine._nativeApi,
      _engine._nativeApi.chapterOpen(_handle, spineIndex),
      'chapter_open',
    );
    final chapter = ChapterSession._(this, handle);
    _chapters.add(chapter);
    return chapter;
  }

  /// Whether this book was disposed directly or by its owning engine.
  bool get isDisposed => _isDisposed;

  /// Disposes the native book and all of its descendant handles.
  ///
  /// Repeated calls are safe. Disposing the owning engine also invalidates the
  /// session without issuing a redundant native call.
  void dispose() {
    _engine._disposeBook(this);
  }

  void _markDisposedByOwner() {
    _isDisposed = true;
    for (final chapter in _chapters.toList(growable: false)) {
      chapter._markDisposedByOwner();
    }
    _chapters.clear();
  }

  void _disposeChapter(ChapterSession chapter) {
    if (chapter._isDisposed) {
      return;
    }
    if (_isDisposed) {
      chapter._markDisposedByOwner();
      return;
    }
    _requireSuccess(
      _engine._nativeApi.handleDispose(chapter._handle),
      'handle_dispose(chapter)',
    );
    chapter._markDisposedByOwner();
    _chapters.remove(chapter);
  }

  void _ensureOpen() {
    if (_isDisposed) {
      throw StateError('BookSession has been disposed.');
    }
  }
}
