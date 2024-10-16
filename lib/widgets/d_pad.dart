import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

class FocusManager extends ChangeNotifier {
  final GlobalKey rootKey = GlobalKey();
  Plane? rootPlane;
  List<Plane> planes = [];

  void addPlane(Plane plane) {
    planes.add(plane);
    notifyListeners();
  }

  Plane? findAdjacentPlane(Plane currentPlane, Direction direction) {
    return _recursivelyFindAdjacentPlane(currentPlane, direction);
  }

  Plane? _recursivelyFindAdjacentPlane(Plane currentPlane, Direction direction,
      {Plane? parentPlane}) {
    int currentIndex = parentPlane?.childPlanes.indexOf(currentPlane) ??
        planes.indexOf(currentPlane);
    List<Plane> searchList = parentPlane?.childPlanes ?? planes;

    Plane? nextPlane;
    switch (direction) {
      case Direction.right:
      case Direction.down:
        nextPlane = (currentIndex < searchList.length - 1)
            ? searchList[currentIndex + 1]
            : null;
        break;
      case Direction.left:
      case Direction.up:
        nextPlane = (currentIndex > 0) ? searchList[currentIndex - 1] : null;
        break;
    }

    if (nextPlane != null) {
      return nextPlane;
    } else if (parentPlane != null) {
      return _recursivelyFindAdjacentPlane(parentPlane, direction,
          parentPlane: parentPlane.parentPlane);
    }

    return null;
  }

  void setRootPlane(Plane plane) {
    rootPlane = plane;
    notifyListeners();
  }

  @override
  void dispose() {
    rootPlane?.dispose();
    for (var plane in planes) {
      plane.dispose();
    }
    super.dispose();
  }
}

class Plane with ChangeNotifier {
  final int rows;
  final int columns;
  final List<List<FocusableElement>> elements;
  final GlobalKey planeKey = GlobalKey();
  FocusableElement? currentFocus;
  FocusManager? focusManager;
  Plane? parentPlane;
  List<Plane> childPlanes = [];

  Plane({required this.rows, required this.columns})
      : elements = List.generate(
          rows,
          (row) => List.generate(
            columns,
            (col) => FocusableElement()..setPosition(row, col),
          ),
        );

  void setFocusManager(FocusManager manager) {
    focusManager = manager;
  }

  void moveFocus(Direction direction) {
    if (currentFocus == null) {
      if (elements.isNotEmpty && elements[0].isNotEmpty) {
        _setFocus(elements[0][0]);
      }
      return;
    }

    FocusableElement? nextElement = findAdjacentElement(direction);
    if (nextElement != null) {
      _setFocus(nextElement);
    } else {
      // Try to cross to another plane
      Plane? adjacentPlane = focusManager?.findAdjacentPlane(this, direction);
      if (adjacentPlane != null) {
        adjacentPlane.receiveFocus(direction, currentFocus!);
      } else {
        // Silently handle reaching the boundary of the entire focus space
        // print('Reached boundary of the entire focus space');
      }
    }
  }

  void receiveFocus(Direction fromDirection, FocusableElement previousFocus) {
    FocusableElement? elementToFocus =
        findElementToFocus(fromDirection, previousFocus);
    if (elementToFocus != null) {
      _setFocus(elementToFocus);
    }
  }

  FocusableElement? findElementToFocus(
      Direction fromDirection, FocusableElement previousFocus) {
    switch (fromDirection) {
      case Direction.right:
        return _findClosestElement(0, previousFocus.row);
      case Direction.left:
        return _findClosestElement(columns - 1, previousFocus.row);
      case Direction.down:
        return _findClosestElement(previousFocus.column, 0);
      case Direction.up:
        return _findClosestElement(previousFocus.column, rows - 1);
    }
  }

  FocusableElement? _findClosestElement(int targetColumn, int targetRow) {
    FocusableElement? closestElement;
    double minDistance = double.infinity;

    for (int row = 0; row < rows; row++) {
      for (int col = 0; col < columns; col++) {
        FocusableElement element = elements[row][col];
        if (element.isRendered) {
          double distance =
              _calculateDistance(row, col, targetRow, targetColumn);
          if (distance < minDistance) {
            minDistance = distance;
            closestElement = element;
          }
        }
      }
    }

    return closestElement;
  }

  double _calculateDistance(int row1, int col1, int row2, int col2) {
    return ((row1 - row2) * (row1 - row2) + (col1 - col2) * (col1 - col2))
        .toDouble();
  }

  FocusableElement? findAdjacentElement(Direction direction) {
    if (currentFocus == null) return null;

    int currentRow = currentFocus!.row;
    int currentCol = currentFocus!.column;

    switch (direction) {
      case Direction.up:
        return _findNextRenderedElement(currentRow - 1, -1, currentCol, 0);
      case Direction.down:
        return _findNextRenderedElement(currentRow + 1, rows, currentCol, 0);
      case Direction.left:
        return _findNextRenderedElement(currentRow, 0, currentCol - 1, -1);
      case Direction.right:
        return _findNextRenderedElement(currentRow, 0, currentCol + 1, columns);
    }
  }

  FocusableElement? _findNextRenderedElement(
      int startRow, int endRow, int startCol, int endCol) {
    int rowStep = startRow < endRow ? 1 : -1;
    int colStep = startCol < endCol ? 1 : -1;

    for (int row = startRow; row != endRow; row += rowStep) {
      for (int col = startCol; col != endCol; col += colStep) {
        if (row >= 0 && row < rows && col >= 0 && col < columns) {
          FocusableElement element = elements[row][col];
          if (element.isRendered) {
            return element;
          }
        }
      }
    }

    return null;
  }

  void addElement(int row, int col, FocusableElement element) {
    if (row < rows && col < columns) {
      elements[row][col] = element;
      element.setPosition(row, col);
    }
  }

  void addChildPlane(Plane plane) {
    childPlanes.add(plane);
    plane.parentPlane = this;
    plane.setFocusManager(focusManager!);
  }

  void removeElement(int row, int col) {
    if (row < rows && col < columns) {
      elements[row][col].dispose();
      elements[row][col] = FocusableElement()..setPosition(row, col);
      if (currentFocus == elements[row][col]) {
        // Find a new element to focus
        currentFocus = null;
        moveFocus(Direction.right); // or any other direction
      }
    }
  }

  void _setFocus(FocusableElement element) {
    if (currentFocus != null) {
      currentFocus!.focusNode.unfocus();
    }
    currentFocus = element;
    element.focusNode.requestFocus();
    notifyListeners();
  }

  @override
  void dispose() {
    for (var row in elements) {
      for (var element in row) {
        element.dispose();
      }
    }

    for (var plane in childPlanes) {
      plane.dispose();
    }
    super.dispose();
  }
}

class FocusableElement {
  final GlobalKey elementKey = GlobalKey();
  final FocusNode focusNode = FocusNode();
  late int row;
  late int column;
  bool isRendered = true;

  void setPosition(int row, int column) {
    this.row = row;
    this.column = column;
  }

  void dispose() {
    focusNode.dispose();
  }
}

enum Direction { up, down, left, right }

class DPadFocusScope extends StatelessWidget {
  final Widget child;
  final FocusManager focusManager;

  const DPadFocusScope({super.key, required this.child, required this.focusManager});

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider.value(
      value: focusManager,
      child: Focus(
        autofocus: true,
        onKey: (node, event) {
          if (event is RawKeyDownEvent) {
            if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
              focusManager.rootPlane?.moveFocus(Direction.up);
              return KeyEventResult.handled;
            } else if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
              focusManager.rootPlane?.moveFocus(Direction.down);
              return KeyEventResult.handled;
            } else if (event.logicalKey == LogicalKeyboardKey.arrowLeft) {
              focusManager.rootPlane?.moveFocus(Direction.left);
              return KeyEventResult.handled;
            } else if (event.logicalKey == LogicalKeyboardKey.arrowRight) {
              focusManager.rootPlane?.moveFocus(Direction.right);
              return KeyEventResult.handled;
            }
          }
          return KeyEventResult.ignored;
        },
        child: child,
      ),
    );
  }
}
