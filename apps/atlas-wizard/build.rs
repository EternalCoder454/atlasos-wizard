use cxx_qt_build::CxxQtBuilder;

fn main() {
    // Generates the C++ for every #[cxx_qt::bridge] listed here and compiles it
    // into the Rust static library. Qt is found through $QMAKE (CMake sets it).
    CxxQtBuilder::new().file("src/backend.rs").build();
}
