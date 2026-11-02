//! ウィンドウ・入力・イベントループ (winit)。
//!
//! 外から見えるのは自前の型だけで、winit の型は出さない。
//!
//! - [`run`] がイベントループを回し、[`App`] に [`Event`] を渡す。
//! - [`Window`] は描画 (`biwa-player-render`) が要る raw-window-handle のトレイトだけを実装する。
//! - [`Waker`] は別のスレッド (wasm の vm) からイベントループを起こす。
//!
//! 描き直しはオンデマンドで、[`App::frame_done`] が「まだ動いているものがある」と答えたときだけ
//! 次のフレームを要求する。何も動いていなければイベントが来るまで眠る (常時 60 fps で回さない)。

use std::fmt;
use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowId;

/// プラットフォームのエラー。
#[derive(Debug, Clone)]
pub struct PlatformError(String);

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlatformError {}

/// ウィンドウの大きさ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSize {
    /// 物理ピクセル。
    pub width: u32,
    pub height: u32,
    /// 論理ピクセル 1 つが物理ピクセルいくつか。
    pub scale: f64,
}

impl WindowSize {
    /// 論理ピクセルでの幅と高さ。
    pub fn logical(&self) -> (f64, f64) {
        (f64::from(self.width) / self.scale, f64::from(self.height) / self.scale)
    }
}

/// アプリに届くイベント。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// 大きさ (か HiDPI の倍率) が変わった。
    Resized(WindowSize),
    /// クリック (か Enter / Space)。ノベルの「進む」。
    Advance,
    /// 描く時が来た。
    Redraw,
    /// [`Waker::wake`] で起こされた。
    Woken,
    /// ウィンドウが閉じられようとしている。
    CloseRequested,
}

/// 最初のウィンドウの設定。
#[derive(Debug, Clone)]
pub struct WindowOptions {
    pub title: String,
    /// 論理ピクセル。
    pub width: f64,
    pub height: f64,
}

/// ウィンドウ。描画に渡すために raw-window-handle のトレイトを実装する。
pub struct Window(winit::window::Window);

impl Window {
    /// 次のフレームを描いてほしい。
    pub fn request_redraw(&self) {
        self.0.request_redraw();
    }

    pub fn size(&self) -> WindowSize {
        let s = self.0.inner_size();
        WindowSize { width: s.width, height: s.height, scale: self.0.scale_factor() }
    }
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Window").finish()
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

/// 別のスレッドからイベントループを起こす。
#[derive(Clone)]
pub struct Waker(EventLoopProxy<()>);

impl Waker {
    pub fn wake(&self) {
        // 送れないのはイベントループが終わったあとで、そのときは起こす必要が無い。
        let _ = self.0.send_event(());
    }
}

/// ループを続けるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// ウィンドウの持ち主。
pub trait App {
    /// ウィンドウができた (描画の準備などをする)。
    fn started(&mut self, window: Arc<Window>) -> Flow;
    /// イベントが来た。
    fn event(&mut self, event: Event) -> Flow;
    /// 溜まったイベントを処理し終えた。動いているものが残っていれば true を返す
    /// (次のフレームを要求する)。
    fn frame_done(&mut self) -> bool;
}

/// イベントループを回す。`make_app` はウィンドウより先に、[`Waker`] を受け取ってアプリを作る。
/// ウィンドウが閉じられるか、アプリが [`Flow::Exit`] を返すと戻る。
pub fn run<A: App>(options: WindowOptions, make_app: impl FnOnce(Waker) -> A) -> Result<(), PlatformError> {
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .map_err(|e| PlatformError(format!("failed to start the event loop: {e}")))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let app = make_app(Waker(event_loop.create_proxy()));
    let mut handler = Handler { options, app, window: None, error: None };
    event_loop
        .run_app(&mut handler)
        .map_err(|e| PlatformError(format!("the event loop failed: {e}")))?;
    handler.error.map_or(Ok(()), Err)
}

struct Handler<A> {
    options: WindowOptions,
    app: A,
    window: Option<Arc<Window>>,
    error: Option<PlatformError>,
}

impl<A: App> Handler<A> {
    fn dispatch(&mut self, event_loop: &ActiveEventLoop, flow: Flow) {
        if flow == Flow::Exit {
            event_loop.exit();
        }
    }
}

impl<A: App> ApplicationHandler for Handler<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = winit::window::Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.options.width, self.options.height))
            .with_min_inner_size(winit::dpi::LogicalSize::new(320.0, 180.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(Window(window)),
            Err(e) => {
                self.error = Some(PlatformError(format!("failed to create a window: {e}")));
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window.clone());
        let flow = self.app.started(window.clone());
        self.dispatch(event_loop, flow);
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let event = match event {
            WindowEvent::CloseRequested => Event::CloseRequested,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let Some(window) = &self.window else { return };
                Event::Resized(window.size())
            }
            WindowEvent::RedrawRequested => Event::Redraw,
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => Event::Advance,
            WindowEvent::KeyboardInput {
                event: KeyEvent { state: ElementState::Pressed, logical_key, repeat: false, .. },
                ..
            } if matches!(logical_key, Key::Named(NamedKey::Enter | NamedKey::Space)) => Event::Advance,
            _ => return,
        };
        let flow = self.app.event(event);
        self.dispatch(event_loop, flow);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
        let flow = self.app.event(Event::Woken);
        self.dispatch(event_loop, flow);
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if self.app.frame_done()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}
