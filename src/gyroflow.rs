#![recursion_limit="4096"]
#![windows_subsystem = "windows"]

use cpp::*;
use qmetaobject::*;
use std::cell::RefCell;

pub use gyroflow_core as core;
pub mod util;
pub mod controller;
pub mod rendering;
pub mod resources;
pub mod ui { pub mod theme; pub mod components { pub mod TimelineGyroChart; } }
pub mod qt_gpu { pub mod qrhi_undistort; }

use crate::core::{lens_profile::LensProfile};
use ui::components::TimelineGyroChart::TimelineGyroChart;
use ui::theme::Theme;

// Things to do before first public preview:
// - Fix ffmpeg GPU acceleration detection and test with different graphic cards
// - Setup CI for packaging for Windows
// - Setup CI for packaging for Mac
// - Port Aphobius 2.0

// TODO: more smoothing algorithms

// TODO: Review offsets interpolation code, it doesn't seem to behave correctly with large offsets
// TODO: base all frames on timestamps, build a mapping frame->timestamp
// TODO: wgpu convert to using textures
// TODO: exporting and loading .gyroflow
// TODO: smoothing presets
// TODO: default lens profile
// TODO: render queue
// TODO: cli interface
// TODO: Calibrator
// TODO: -- auto upload of lens profiles to a central database (with a checkbox)
// TODO: -- Save camera model with calibration and later load lens profile automatically
// TODO: -- Save frame readout time
// TODO: -- Save lens parameters (gopro: linear, wide, narrow, also hypersmooth on/off), sony: lens model and zoom position
// TODO: -- Allow for multiple zoom values, could be interpolated later (Sony)
// TODO: -- Recommended output size (eg. 4:3 to 16:9)
// TODO: UI: activeFocus indicators
// TODO: timeline panning
// TODO: add lens distortion back after stabilization
// TODO: hyperlapse mode
// TODO: Setup CI for packaging for Linux
// TODO: Setup CI for packaging for Android
// TODO: Setup CI for packaging for iOS
// TODO: drop mutliple files at once (video, lens profile, gyro data)
// TODO: add elapsed and remaining times when rendering
// TODO: add vertical labels and scale to gyro chart
// TODO: When rendering, it should be possible to "minimize" the status and continue to work. 
// TODO: keyframes for stabilization params
// TODO: detect imu orientation automatically, basically try all combinations for a closest match to OF
// TODO: mask for optical flow
// TODO: Add cache for the undistortion if the video is not playing
// TODO: OpenFX plugin
// TODO: Adobe plugin
// TODO: save panel sizes, menu opened states and window dimensions
// TODO: audio slightly off sync when using exporting trimmed video

cpp! {{
    #include <QQuickStyle>
    #include <QQuickWindow>
    #include <QQmlContext>
    #include <QtGui/QGuiApplication>
    #include <QIcon>

    #include "src/ui_live_reload.cpp"

    #ifdef Q_OS_ANDROID
    #   include <QtCore/private/qandroidextras_p.h>
    #endif
}}

fn entry() {
    let ui_live_reload = false;

    #[cfg(target_os = "windows")]
    /*if std::env::args().any(|x| x == "--console")*/ {
        unsafe {
            winapi::um::wincon::AttachConsole(winapi::um::wincon::ATTACH_PARENT_PROCESS);
        }
    }

    simplelog::TermLogger::init(simplelog::LevelFilter::Debug, simplelog::ConfigBuilder::new()
        .add_filter_ignore_str("mp4parse")
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .add_filter_ignore_str("akaze")
        .add_filter_ignore_str("ureq")
        .add_filter_ignore_str("rustls")
        .build(), simplelog::TerminalMode::Mixed, simplelog::ColorChoice::Auto).unwrap();

    qmetaobject::log::init_qt_to_rust();

    crate::resources::rsrc();
    qml_video_rs::register_qml_types();
    qml_register_type::<TimelineGyroChart>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineGyroChart"));

    // return rendering::test();

    let icons_path = if ui_live_reload {
        QString::from(format!("{}/resources/icons/", env!("CARGO_MANIFEST_DIR")))
    } else {
        QString::from(":/resources/icons/")
    };
    cpp!(unsafe [icons_path as "QString"] {
        QGuiApplication::setOrganizationName("Gyroflow");
        QGuiApplication::setOrganizationDomain("gyroflow.xyz");
        QGuiApplication::setApplicationName("Gyroflow");

        QQuickStyle::setStyle("Material");
        QIcon::setThemeName(QStringLiteral("Gyroflow"));
        QIcon::setThemeSearchPaths(QStringList() << icons_path);

        // QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
        // QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan);
    });

    let ctl = RefCell::new(controller::Controller::new());
    let ctlpinned = unsafe { QObjectPinned::new(&ctl) };

    let calib_ctl = RefCell::new(controller::Controller::new());
    let calib_ctlpinned = unsafe { QObjectPinned::new(&calib_ctl) };

    let theme = RefCell::new(Theme::default());
    let themepinned = unsafe { QObjectPinned::new(&theme) };

    let mut engine = QmlEngine::new();
    let dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; });
    engine.set_property("dpiScale".into(), QVariant::from(dpi));
    engine.set_property("version".into(), QString::from(env!("CARGO_PKG_VERSION")).into());
    engine.set_property("isOpenGl".into(), QVariant::from(false));
    engine.set_object_property("main_controller".into(), ctlpinned);
    engine.set_object_property("calib_controller".into(), calib_ctlpinned);
    engine.set_object_property("theme".into(), themepinned);
    theme.borrow_mut().engine_ptr = Some(&mut engine as *mut _);
    theme.borrow().set_theme("dark".into());

    // Get camera profiles list
    let lens_profiles: QVariantList = LensProfile::get_profiles_list().unwrap_or_default().into_iter().map(QString::from).collect();
    engine.set_property("lensProfilesList".into(), QVariant::from(lens_profiles));

    // Get smoothing algorithms
    engine.set_property("smoothingAlgorithms".into(), QVariant::from(ctl.borrow().get_smoothing_algs()));

    let engine_ptr = engine.cpp_ptr();

    // Load main UI
    if !ui_live_reload {
        engine.load_file("qrc:/src/ui/main_window.qml".into());
    } else {
        engine.load_file(format!("{}/src/ui/main_window.qml", env!("CARGO_MANIFEST_DIR")).into());
        let ui_path = QString::from(format!("{}/src/ui", env!("CARGO_MANIFEST_DIR")));
        cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *", ui_path as "QString"] { init_live_reload(engine_ptr, ui_path); });
    }

    let is_opengl = cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *"] -> bool as "bool" {
        auto wnd = qobject_cast<QQuickWindow *>(engine_ptr->rootObjects().first());
        bool isOpenGl = wnd->rendererInterface()->graphicsApi() == QSGRendererInterface::OpenGLRhi;
        engine_ptr->rootContext()->setContextProperty("isOpenGl", QVariant::fromValue(isOpenGl));
        wnd->setIcon(QIcon(":/resources/icon.png"));
        #ifdef Q_OS_ANDROID
            QtAndroidPrivate::requestPermission(QtAndroidPrivate::Storage).result();
        #endif
        return isOpenGl;
    });
    ctl.borrow_mut().stabilizer.params.write().framebuffer_inverted = is_opengl;
    calib_ctl.borrow_mut().stabilizer.params.write().framebuffer_inverted = is_opengl;

    rendering::init().unwrap();

    engine.exec();
}


#[no_mangle]
#[cfg(target_os = "android")]
pub extern fn main(_argc: i32, _argv: *mut *mut i8) -> i32 {
    entry();
    0
}

#[cfg(not(target_os = "android"))]
fn main() {
    entry();
}
