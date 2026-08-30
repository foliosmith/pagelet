import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../lib/pagelet_reader.dart';

void main() {
  final libraryPath = Platform.environment['PAGELET_LIBRARY_PATH'];
  final bookPath = Platform.environment['PAGELET_READER_EPUB_PATH'];

  testWidgets(
    'renders the first real EPUB page in a swipeable PageView',
    (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          home: PageletReader(
            libraryPath: libraryPath!,
            bookPath: bookPath!,
          ),
        ),
      );

      for (var attempt = 0;
          attempt < 100 && find.byType(PageView).evaluate().isEmpty;
          attempt += 1) {
        await tester.pump(const Duration(milliseconds: 50));
      }

      final errors = tester
          .widgetList<SelectableText>(find.byType(SelectableText))
          .map((widget) => widget.data)
          .whereType<String>()
          .join('\n');
      expect(find.byType(PageView), findsOneWidget, reason: errors);
      expect(find.byType(CustomPaint), findsWidgets);
      expect(find.byKey(const ValueKey<String>('page-0')), findsOneWidget);
      expect(tester.takeException(), isNull);
      final pageView = tester.widget<PageView>(find.byType(PageView));
      expect(pageView.scrollDirection, Axis.horizontal);
      expect(pageView.physics, isNot(isA<NeverScrollableScrollPhysics>()));
    },
    skip: libraryPath == null || bookPath == null,
  );
}
