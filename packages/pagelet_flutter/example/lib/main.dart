import 'package:flutter/material.dart';

import 'pagelet_reader.dart';

const _libraryPath = String.fromEnvironment('PAGELET_LIBRARY_PATH');
const _bookPath = String.fromEnvironment('PAGELET_BOOK_PATH');

void main() {
  runApp(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      home: _libraryPath.isEmpty || _bookPath.isEmpty
          ? const _SetupInstructions()
          : Scaffold(
              body: SafeArea(
                child: PageletReader(
                  libraryPath: _libraryPath,
                  bookPath: _bookPath,
                ),
              ),
            ),
    ),
  );
}

final class _SetupInstructions extends StatelessWidget {
  const _SetupInstructions();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      body: Center(
        child: Padding(
          padding: EdgeInsets.all(24),
          child: SelectableText(
            'Pass PAGELET_LIBRARY_PATH and PAGELET_BOOK_PATH with '
            '--dart-define to open an EPUB.',
          ),
        ),
      ),
    );
  }
}
