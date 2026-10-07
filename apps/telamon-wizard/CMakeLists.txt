cmake_minimum_required(VERSION 3.24)
project(telamon-wizard VERSION 0.2.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
find_package(Qt6 6.11 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)
find_package(KF6CoreAddons REQUIRED)
find_package(KF6DBusAddons REQUIRED)
find_package(KF6WindowSystem REQUIRED)
find_package(Corrosion REQUIRED)

set(WORKSPACE_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../..)

# Rust library (CXX-Qt QObjects, atlas-framework, wizard-core). cxx-qt-build
# finds Qt via $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${WORKSPACE_DIR}/Cargo.toml
    CRATES telamon-wizard LOCKED)
corrosion_set_env_vars(telamon_wizard QMAKE=${QMAKE6})

# Telamon.Ui, the shared Telamon look, is installed like Kirigami (telamon-ui, from
# atlas-framework): QML imports it and nothing links it. qmlcachegen and
# qmllint read its types from the installed copy.
if(NOT EXISTS "${QT6_INSTALL_PREFIX}/${QT6_INSTALL_QML}/Telamon/Ui/qmldir")
    message(FATAL_ERROR "Telamon.Ui is not installed: install telamon-ui (atlas-framework)")
endif()

qt_add_executable(telamon-wizard-app cpp/main.cpp)
set_target_properties(telamon-wizard-app PROPERTIES OUTPUT_NAME telamon-wizard)

# qmlcachegen compiles every QML file to C++ at build time; the app reads no
# .qml from disk at run time.
qt_add_qml_module(telamon-wizard-app
    URI net.eterneon.telamon.wizard
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/ErrorText.qml
        qml/WizardPage.qml
        qml/WizardOnboarding.qml
        qml/WizardStep.qml
        qml/WizardChoiceCard.qml
        qml/WizardThemePreview.qml
        qml/WizardLogo.qml
        qml/WizardPicker.qml
        qml/WelcomePage.qml
        qml/LanguagePage.qml
        qml/KeyboardPage.qml
        qml/WifiPage.qml
        qml/TimeZonePage.qml
        qml/AccountPage.qml
        qml/HostnamePage.qml
        qml/AppearancePage.qml
        qml/PrivacyPage.qml
        qml/FinishPage.qml
        qml/FirstLoginPage.qml
)

target_link_libraries(telamon-wizard-app PRIVATE
    telamon_wizard
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets
    KF6::CoreAddons KF6::DBusAddons KF6::WindowSystem
    # wizard-core calls libxcrypt and libpwquality; a static Rust library carries no link flags.
    crypt pwquality)

install(TARGETS telamon-wizard-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.telamon.wizard.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
