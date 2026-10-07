pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Shapes
import Telamon.Ui

// A small picture of the Telamon OS desktop in one colour scheme, drawn with
// shapes (so it is sharp at any scale, and costs no image): the wallpaper, the
// menu bar's islands, a window in the Telamon.Ui look and the dock. The colours
// are the ones TelamonStyle derives from the TelamonLight and TelamonDark
// schemes (the window colour, tinted toward violet, and its tonal steps), and
// `accent` is the accent colour the user is picking. The picture is 16:10 and
// is laid out in 160 x 100 units, every edge snapped to a device pixel.
Item {
    id: scene

    property bool dark: false
    property color accent: "#6858E2" // telamon-lint: allow-raw

    // TelamonLight.colors and TelamonDark.colors: Window background and text.
    readonly property color _window: scene.dark ? "#211E38" : "#F3F2FA" // telamon-lint: allow-raw
    readonly property color _fg: scene.dark ? "#EEECFA" : "#1B1748" // telamon-lint: allow-raw
    readonly property color _tint: scene.dark ? "#8A7AF4" : "#6858E2" // telamon-lint: allow-raw
    readonly property color _white: "#FFFFFF" // telamon-lint: allow-raw
    // The same steps as TelamonStyle's, from those two colours.
    readonly property color _base: TelamonStyle.mix(scene._window, scene._tint, scene.dark ? 0.06 : 0.045)
    readonly property color _surface: scene.dark ? TelamonStyle.mix(scene._base, scene._white, 0.045) : TelamonStyle.mix(scene._base, scene._white, 0.6)
    readonly property color _raised: scene.dark ? TelamonStyle.mix(scene._base, scene._white, 0.085) : TelamonStyle.mix(scene._base, scene._white, 0.85)
    readonly property color _control: scene.dark ? TelamonStyle.mix(scene._base, scene._white, 0.065) : TelamonStyle.mix(scene._base, scene._fg, 0.055)
    readonly property color _line: TelamonStyle.alpha(scene._fg, 0.08)
    readonly property color _edge: TelamonStyle.alpha(scene._fg, 0.22)
    readonly property color _muted: TelamonStyle.alpha(scene._fg, 0.55)
    readonly property color _onAccent: {
        const c = scene.accent;
        return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b > 0.6 ? "#14121F" : "#FFFFFF"; // telamon-lint: allow-raw
    }

    // Units to pixels, kept on the device's pixel grid.
    readonly property real _u: width / 160
    readonly property real _dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    function px(v: real): real {
        return Math.round(v * scene._u * scene._dpr) / scene._dpr;
    }

    implicitWidth: 160
    implicitHeight: 100
    Accessible.ignored: true

    // A soft round glow: a radial gradient that fades out before its edge.
    component Glow: Shape {
        id: glow
        property color color: scene._white
        property real cx
        property real cy
        property real reach
        x: scene.px(cx - reach)
        y: scene.px(cy - reach)
        width: scene.px(reach * 2)
        height: width
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeWidth: -1
            fillGradient: RadialGradient {
                centerX: glow.width / 2
                centerY: glow.height / 2
                centerRadius: glow.width / 2
                focalX: centerX
                focalY: centerY
                GradientStop {
                    position: 0
                    color: glow.color
                }
                GradientStop {
                    position: 1
                    color: TelamonStyle.alpha(glow.color, 0)
                }
            }
            startX: 0
            startY: 0
            PathLine {
                x: glow.width
                y: 0
            }
            PathLine {
                x: glow.width
                y: glow.height
            }
            PathLine {
                x: 0
                y: glow.height
            }
        }
    }
    // A rounded bar or block in scene units.
    component Block: Rectangle {
        property real ux
        property real uy
        property real uw
        property real uh
        property real ur: 0
        x: scene.px(ux)
        y: scene.px(uy)
        width: scene.px(uw)
        height: scene.px(uh)
        radius: ur < 0 ? height / 2 : Math.min(scene.px(ur), height / 2)
        antialiasing: true
    }

    // Wallpaper: the cherry tree's colours, as soft glows over a sky.
    Rectangle {
        id: wall
        anchors.fill: parent
        radius: TelamonStyle.radiusLarge
        gradient: Gradient {
            GradientStop {
                position: 0
                color: scene.dark ? "#171548" : "#86AECF" // telamon-lint: allow-raw
            }
            GradientStop {
                position: 0.7
                color: scene.dark ? "#4A2263" : "#DDB0C4" // telamon-lint: allow-raw
            }
            GradientStop {
                position: 1
                color: scene.dark ? "#2A1038" : "#C98AA6" // telamon-lint: allow-raw
            }
        }
        Glow {
            color: scene.dark ? "#E8559A" : "#F2658F" // telamon-lint: allow-raw
            cx: 52
            cy: 42
            reach: 42
        }
        Glow {
            color: scene.dark ? "#F7A57C" : "#FBC9A0" // telamon-lint: allow-raw
            cx: 126
            cy: 32
            reach: 32
        }
        Glow {
            color: scene.dark ? "#B24CC4" : "#F58FB4" // telamon-lint: allow-raw
            cx: 96
            cy: 66
            reach: 34
        }
    }

    // The menu bar: three islands, each as wide as what it holds.
    Repeater {
        model: [
            {
                x: 4,
                w: 33
            },
            {
                x: 66,
                w: 28
            },
            {
                x: 123,
                w: 33
            }
        ]
        Item {
            id: island
            required property var modelData
            required property int index
            Block {
                ux: island.modelData.x
                uy: 3
                uw: island.modelData.w
                uh: 7
                ur: -1
                color: TelamonStyle.alpha(scene._raised, 0.82)
            }
            // The Telamon OS menu, then the open app's menus.
            Block {
                visible: island.index === 0
                ux: island.modelData.x + 3
                uy: 4.5
                uw: 4
                uh: 4
                ur: -1
                color: scene.accent
            }
            Repeater {
                model: island.index === 0 ? 3 : (island.index === 1 ? 1 : 4)
                Block {
                    id: tick
                    required property int index
                    ux: island.index === 0 ? island.modelData.x + 10 + tick.index * 7.5 : (island.index === 1 ? island.modelData.x + 7 : island.modelData.x + 6 + tick.index * 6.5)
                    uy: island.index === 2 ? 5.0 : 5.6
                    uw: island.index === 0 ? 5.5 : (island.index === 1 ? 14 : 3.2)
                    uh: island.index === 2 ? 3.2 : 1.8
                    ur: -1
                    color: scene._muted
                }
            }
        }
    }

    // The window: its colour, the header bar and the sidebar (their corners
    // follow the window's), the page, then the outline on top.
    Block {
        ux: 24
        uy: 17
        uw: 100
        uh: 60
        ur: 2.6
        color: scene._base
    }
    Block {
        ux: 24
        uy: 17
        uw: 100
        uh: 7
        ur: 2.6
        color: scene._surface
    }
    Block {
        ux: 24
        uy: 20.5
        uw: 100
        uh: 3.5
        color: scene._surface
    }
    Block {
        ux: 24
        uy: 24
        uw: 27
        uh: 53
        ur: 2.6
        color: scene._surface
    }
    Block {
        ux: 24
        uy: 24
        uw: 27
        uh: 4
        color: scene._surface
    }
    Block {
        ux: 47
        uy: 24
        uw: 4
        uh: 53
        color: scene._surface
    }
    Rectangle {
        x: scene.px(24)
        y: scene.px(24)
        width: scene.px(100)
        height: 1
        color: scene._line
    }
    Rectangle {
        x: scene.px(51)
        y: scene.px(24)
        width: 1
        height: scene.px(53)
        color: scene._line
    }
    // Header: title and window buttons.
    Block {
        ux: 66
        uy: 19.1
        uw: 18
        uh: 1.8
        ur: -1
        color: scene._fg
        opacity: 0.8
    }
    Repeater {
        model: 3
        Block {
            required property int index
            ux: 107 + index * 4.4
            uy: 18.7
            uw: 2.6
            uh: 2.6
            ur: -1
            color: index === 2 ? TelamonStyle.alpha(scene._fg, 0.28) : scene._muted
        }
    }
    // Sidebar: the chosen row, then four more.
    Block {
        ux: 27
        uy: 27
        uw: 21
        uh: 5.6
        ur: 1.2
        color: TelamonStyle.alpha(scene.accent, scene.dark ? 0.24 : 0.16)
    }
    Block {
        ux: 30
        uy: 29
        uw: 4
        uh: 1.8
        ur: -1
        color: scene.accent
    }
    Block {
        ux: 36
        uy: 29
        uw: 9
        uh: 1.8
        ur: -1
        color: scene._fg
    }
    Repeater {
        model: 4
        Item {
            id: navRow
            required property int index
            Block {
                ux: 30
                uy: 36.6 + navRow.index * 5.8
                uw: 4
                uh: 1.8
                ur: -1
                color: scene._muted
            }
            Block {
                ux: 36
                uy: 36.6 + navRow.index * 5.8
                uw: 7 + (navRow.index % 2) * 2.5
                uh: 1.8
                ur: -1
                color: scene._muted
            }
        }
    }
    // Page title and a section of three rows.
    Block {
        ux: 56
        uy: 28
        uw: 30
        uh: 2.6
        ur: -1
        color: scene._fg
        opacity: 0.9
    }
    Rectangle {
        id: card
        x: scene.px(56)
        y: scene.px(34)
        width: scene.px(64)
        height: scene.px(22.5)
        radius: scene.px(1.6)
        color: scene._surface
        border.width: 1
        border.color: scene._line
        Repeater {
            model: 3
            Item {
                id: row
                required property int index
                Rectangle {
                    visible: row.index > 0
                    x: scene.px(3)
                    y: scene.px(row.index * 7.5)
                    width: card.width - 2 * scene.px(3)
                    height: 1
                    color: scene._line
                }
                Block {
                    x: scene.px(3)
                    y: scene.px(row.index * 7.5 + 2.8)
                    uw: 20 + row.index * 4
                    uh: 1.8
                    ur: -1
                    color: scene._fg
                    opacity: 0.8
                }
                // A switch: on for the first row.
                Block {
                    x: card.width - scene.px(3 + 7)
                    y: scene.px(row.index * 7.5 + 1.9)
                    uw: 7
                    uh: 3.6
                    ur: -1
                    color: row.index === 0 ? scene.accent : scene._control
                    border.width: row.index === 0 ? 0 : 1
                    border.color: scene._edge
                    Rectangle {
                        width: parent.height - scene.px(1)
                        height: width
                        radius: width / 2
                        y: (parent.height - height) / 2
                        x: row.index === 0 ? parent.width - width - scene.px(0.5) : scene.px(0.5)
                        color: row.index === 0 ? scene._onAccent : scene._muted
                    }
                }
            }
        }
    }
    // A primary and a default button.
    Block {
        ux: 56
        uy: 61
        uw: 17
        uh: 5.6
        ur: 1.2
        color: scene.accent
        Block {
            x: (parent.width - width) / 2
            y: (parent.height - height) / 2
            uw: 8.5
            uh: 1.6
            ur: -1
            color: scene._onAccent
        }
    }
    Block {
        ux: 75
        uy: 61
        uw: 14
        uh: 5.6
        ur: 1.2
        color: scene._control
        border.width: 1
        border.color: scene._edge
        Block {
            x: (parent.width - width) / 2
            y: (parent.height - height) / 2
            uw: 7
            uh: 1.6
            ur: -1
            color: scene._fg
            opacity: 0.8
        }
    }
    // The outline.
    Block {
        ux: 24
        uy: 17
        uw: 100
        uh: 60
        ur: 2.6
        color: "transparent"
        border.width: 1
        border.color: scene._edge
    }

    // The dock.
    Block {
        id: dock
        ux: 49
        uy: 83.5
        uw: 62
        uh: 11.5
        ur: 4
        color: TelamonStyle.alpha(scene._raised, scene.dark ? 0.5 : 0.62)
        border.width: 1
        border.color: TelamonStyle.alpha(scene._fg, scene.dark ? 0.16 : 0.1)
        Repeater {
            model: ["accent", "#4C9BE8", "#F29A4B", scene.dark ? "#6E6D90" : "#3B3A4C", "#9A6BE0", "#8B93A8"] // telamon-lint: allow-raw
            Block {
                id: app
                required property var modelData
                required property int index
                x: scene.px(2.4 + index * 9.6)
                y: scene.px(2.2)
                uw: 7.4
                uh: 7.4
                ur: 2
                color: modelData === "accent" ? scene.accent : modelData
                Block {
                    visible: app.index === 1
                    x: (parent.width - width) / 2
                    y: parent.height + scene.px(0.9)
                    uw: 3
                    uh: 0.8
                    ur: -1
                    color: scene._fg
                    opacity: 0.7
                }
            }
        }
    }
}
