import 'package:fluent_ui/fluent_ui.dart';
import 'package:mesh_gradient/mesh_gradient.dart';

class ThemeGradient extends StatelessWidget {
  const ThemeGradient({super.key});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
        width: 24,
        height: 48,
        child: MeshGradient(
          points: [
            MeshGradientPoint(
              position: const Offset(
                0.2,
                0.6,
              ),
              color: const Color.fromARGB(255, 251, 0, 105),
            ),
            MeshGradientPoint(
              position: const Offset(
                0.4,
                0.5,
              ),
              color: const Color.fromARGB(255, 69, 18, 255),
            ),
            MeshGradientPoint(
              position: const Offset(
                0.7,
                0.4,
              ),
              color: const Color.fromARGB(255, 0, 255, 198),
            ),
            MeshGradientPoint(
              position: const Offset(
                0.4,
                0.9,
              ),
              color: const Color.fromARGB(255, 64, 255, 0),
            ),
          ],
          options: MeshGradientOptions(),
        ));
  }
}
