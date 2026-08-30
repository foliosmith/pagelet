import 'package:flutter/material.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  const decoder = PageSceneDecoder();
  runApp(
    MaterialApp(
      home: Scaffold(
        body: Center(
          child: Text(decoder.runtimeType.toString()),
        ),
      ),
    ),
  );
}
