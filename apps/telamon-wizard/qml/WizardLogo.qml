import QtQuick
import QtQuick.Shapes

// The Telamon mark (branding/source/telamon-mark.svg of the OS image), drawn
// with shapes: the welcome page's logo when the icon theme has no `telamon`
// icon, as in a container or a minimal session. Square; `size` is its side.
Item {
    id: mark

    property real size: 96
    implicitWidth: mark.size
    implicitHeight: mark.size
    Accessible.ignored: true

    Shape {
        width: 1024
        height: 1024
        scale: mark.size / 1024
        transformOrigin: Item.TopLeft
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeWidth: -1
            fillGradient: LinearGradient {
                x1: 220
                y1: 168
                x2: 340
                y2: 872
                GradientStop {
                    position: 0
                    color: "#C3B8FF" // telamon-lint: allow-raw
                }
                GradientStop {
                    position: 1
                    color: "#9C8CF8" // telamon-lint: allow-raw
                }
            }
            PathSvg {
                path: "M411.9 197.6Q424 168 456 168L590 168Q600 168 596.2 177.3L324.1 842.4Q312 872 280 872L168 872Q136 872 148.1 842.4Z"
            }
        }
        ShapePath {
            strokeWidth: -1
            fillGradient: LinearGradient {
                x1: 600
                y1: 168
                x2: 660
                y2: 872
                GradientStop {
                    position: 0
                    color: "#251B72" // telamon-lint: allow-raw
                }
                GradientStop {
                    position: 0.3
                    color: "#7262EA" // telamon-lint: allow-raw
                }
                GradientStop {
                    position: 1
                    color: "#6252DD" // telamon-lint: allow-raw
                }
            }
            PathSvg {
                path: "M445.3 191.9Q424 168 456 168L590 168Q600 168 603.8 177.3L875.9 842.4Q888 872 856 872L744 872Q712 872 699.9 842.4L527.1 420.1Q512 383.1 527.1 346.1L544.7 303.1Z"
            }
        }
        ShapePath {
            strokeWidth: -1
            fillGradient: LinearGradient {
                x1: 434.7
                y1: 572
                x2: 644.5
                y2: 707
                GradientStop {
                    position: 0
                    color: "#8A7AF4" // telamon-lint: allow-raw
                }
                GradientStop {
                    position: 1
                    color: "#7A69EE" // telamon-lint: allow-raw
                }
            }
            PathSvg {
                path: "M346.7 572L677.3 572L732.5 707L291.5 707Z"
            }
        }
    }
}
