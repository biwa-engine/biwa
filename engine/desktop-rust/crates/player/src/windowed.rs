//! ウィンドウを出して動かす。
//!
//! 部品をつなぐだけである:
//! - `platform` のイベントループから来るイベントを、エンジン (`core`) への操作に直す
//! - `vm` からの知らせ (syscall) をエンジンに当て、止まる syscall の返事を保留する
//! - エンジンの画像の読み込みの依頼を `image` に渡し、読めた画像をエンジン (大きさ) と `render` (テクスチャ) に渡す
//! - 描き直しが要るときだけ、エンジンの描画の指示を `render` で描く (文字は `text`)
//!
//! ゲームは決まった大きさ (論理ピクセル。既定は 1280x720) の画面として組み、ウィンドウに収まるよう
//! 縦横比を保って拡大・縮小する。余りは黒い帯 (レターボックス)。ウィンドウの大きさや全画面のモニタの
//! 比が違っても、レイアウトは変わらない。ウィンドウの縦横比も、できる OS (Windows / macOS) では固定する。

use std::sync::Arc;
use std::time::{Duration, Instant};

use biwa_player_core::engine::{CallStatus, Click, Engine};
use biwa_player_core::geometry::{Rect, Viewport};
use biwa_player_core::image::{DecodedImage, ImageHandle};
use biwa_player_image::ImageLoader;
use biwa_player_layout::TaffyLayouter;
use biwa_player_platform::{App, Event, Flow, Window, WindowOptions, WindowSize};
use biwa_player_render::{FrameStatus, Renderer};
use biwa_player_text::TextSystem;
use biwa_player_vm::{CallReply, HostCommand, Vm, VmEvent, VmOptions};

use crate::options::Options;

/// 1 フレームで進める時間の上限。描画が詰まったときに文字送りが一気に飛ばないように。
const MAX_FRAME_STEP: Duration = Duration::from_millis(100);

pub fn run(options: &Options, wasm: Vec<u8>) -> Result<(), String> {
    let screen = options.screen;
    let window = WindowOptions {
        title: options.title.clone(),
        width: f64::from(screen.0),
        height: f64::from(screen.1),
        aspect_ratio: Some(screen),
    };
    let collector = options.collector;
    let trace = options.trace;
    let assets = options.assets.clone();
    if assets.is_none() {
        eprintln!("[biwa] no assets directory found; images will not be shown (pass --assets)");
    }
    let mut spawn_error = None;
    let result = biwa_player_platform::run(window, |waker| {
        let images = assets.and_then(|dir| {
            let waker = waker.clone();
            ImageLoader::new(dir, move || waker.wake())
                .map_err(|e| eprintln!("[biwa] failed to start the image loader: {e}"))
                .ok()
        });
        let vm = Vm::spawn(wasm, VmOptions { collector }, move || waker.wake())
            .map_err(|e| spawn_error = Some(format!("failed to start the vm thread: {e}")))
            .ok();
        Player::new(vm, images, screen, trace)
    });
    if let Some(e) = spawn_error {
        return Err(e);
    }
    result.map_err(|e| e.to_string())
}

struct Player {
    vm: Option<Vm>,
    images: Option<ImageLoader>,
    /// 読めたが、描画の準備 (ウィンドウ) の前に届いた画像。
    unuploaded: Vec<(ImageHandle, DecodedImage)>,
    engine: Engine,
    text: TextSystem,
    layouter: TaffyLayouter,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    size: Option<WindowSize>,
    /// ゲームの画面の大きさ (論理ピクセル)。
    screen: (u32, u32),
    /// 描けないまま (ウィンドウが見えていない) の変更がある。見えるようになったら描く。
    stale: bool,
    /// 返事を保留している止まる syscall。
    pending: Option<CallReply>,
    /// 最後にエンジン時計を進めた時刻。動いているものが無い間は `None`。
    last_tick: Option<Instant>,
    trace: bool,
}

impl Player {
    fn new(vm: Option<Vm>, images: Option<ImageLoader>, screen: (u32, u32), trace: bool) -> Self {
        Self {
            vm,
            images,
            unuploaded: Vec::new(),
            engine: Engine::new(),
            text: TextSystem::new(),
            layouter: TaffyLayouter::new(),
            window: None,
            renderer: None,
            size: None,
            screen,
            stale: false,
            pending: None,
            last_tick: None,
            trace,
        }
    }

    /// vm から届いている知らせを全部処理する。
    fn drain_vm(&mut self) -> Flow {
        loop {
            let Some(event) = self.vm.as_ref().and_then(Vm::try_recv) else {
                return Flow::Continue;
            };
            match event {
                VmEvent::Batch(calls) => {
                    for call in calls {
                        if self.trace {
                            println!("  {call:?}");
                        }
                        if let Err(e) = self.engine.apply(call) {
                            eprintln!("[biwa] {e}");
                        }
                    }
                    self.request_images();
                    self.release_handlers();
                }
                VmEvent::Call(call, reply) => {
                    if self.trace {
                        println!("  {call:?}");
                    }
                    match self.engine.begin_call(&call) {
                        CallStatus::Done => reply.done(),
                        CallStatus::Pending => self.pending = Some(reply),
                    }
                }
                // UI が出揃った。scene は SceneStartButton が押されたら始める。
                VmEvent::AwaitingScene => {}
                VmEvent::Exited(_) => println!("[biwa] the scene finished"),
                VmEvent::Failed(message) => {
                    eprintln!("error: {message}");
                    self.vm = None;
                }
            }
            self.request_redraw();
        }
    }

    /// 手放してよくなった関数の番号を vm に伝える (Element が消えたとき)。
    fn release_handlers(&mut self) {
        let released = self.engine.take_released_handlers();
        if !released.is_empty()
            && let Some(vm) = &self.vm
        {
            vm.send(HostCommand::Release(released));
        }
    }

    /// エンジンの画像の読み込みの依頼を、読む者に渡す。
    fn request_images(&mut self) {
        let requests = self.engine.take_image_requests();
        if let Some(images) = &self.images {
            for request in requests {
                images.request(request);
            }
        }
    }

    /// 読めた画像を、エンジン (大きさ) と描画 (テクスチャ) に渡す。
    fn drain_images(&mut self) {
        while let Some((image, result)) = self.images.as_ref().and_then(ImageLoader::try_recv) {
            match result {
                Ok(decoded) => {
                    self.engine.image_loaded(image, decoded.width, decoded.height);
                    self.unuploaded.push((image, decoded));
                }
                Err(e) => eprintln!("[biwa] {e}"),
            }
        }
        if let Some(renderer) = &mut self.renderer {
            for (image, decoded) in self.unuploaded.drain(..) {
                renderer.upload_image(image, &decoded);
            }
        }
        self.request_redraw();
    }

    /// 保留中の止まる syscall が終わっていれば返事をする。
    fn settle_call(&mut self) {
        if self.engine.take_finished_call()
            && let Some(reply) = self.pending.take()
        {
            reply.done();
        }
    }

    fn redraw(&mut self) {
        // 動いているものがある間だけ時計を進める。止まっていた時間は流さない。
        let now = Instant::now();
        if let Some(last) = self.last_tick {
            let step = (now - last).min(MAX_FRAME_STEP);
            self.engine.update(step.as_secs_f32() * 1000.0);
        }
        self.last_tick = self.engine.is_animating().then_some(now);
        self.settle_call();

        let (Some(renderer), Some(size)) = (&mut self.renderer, self.size) else {
            return;
        };
        let fit = Letterbox::new(self.screen, size);
        let viewport = Viewport::new(self.screen.0 as f32, self.screen.1 as f32, fit.scale);
        let list = self.engine.frame(viewport, &mut self.text, &mut self.layouter);
        let status = renderer.render(&list, &mut self.text, fit.area);
        // Windows は隠したまま作ったウィンドウを、最初に描いたところで見せる。
        if let Some(window) = &self.window {
            window.show();
        }
        match status {
            Ok(FrameStatus::Presented) => self.stale = false,
            // 見えるようになったら (`Event::Shown`) 描く。
            Ok(FrameStatus::Hidden) => self.stale = true,
            Ok(FrameStatus::Retry) => self.request_redraw(),
            Err(e) => eprintln!("[biwa] {e}"),
        }
    }

    /// クリックや「進む」のあと。止まっている syscall を進め、描き直す。
    fn after_input(&mut self) {
        self.settle_call();
        self.last_tick = None;
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl App for Player {
    fn started(&mut self, window: Arc<Window>) -> Flow {
        if self.vm.is_none() {
            // vm を起動できなかった (理由は `run` が返す)。
            return Flow::Exit;
        }
        let size = window.size();
        match Renderer::new(window.clone(), size.width, size.height) {
            Ok(renderer) => {
                println!("[biwa] rendering with {}", renderer.adapter());
                self.renderer = Some(renderer);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return Flow::Exit;
            }
        }
        self.size = Some(size);
        self.window = Some(window);
        self.drain_images();
        let flow = self.drain_vm();
        // 最初のフレームは描き直しの知らせを待たずに描く。Windows は隠したまま作ったウィンドウをここで見せる
        // (隠れているウィンドウには描き直しの知らせが来ないことがある)。
        self.redraw();
        flow
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Resized(size) => {
                self.size = Some(size);
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.request_redraw();
            }
            Event::Click { x, y } => {
                // ウィンドウの論理ピクセルから、ゲームの画面の論理ピクセルへ。
                let position = match self.size {
                    Some(size) => Letterbox::new(self.screen, size).to_screen(x, y, size.scale as f32),
                    None => Some((x, y)),
                };
                let Some((x, y)) = position else {
                    // 帯の上。どの要素にも当たらないクリックと同じく「進む」。
                    self.engine.advance();
                    self.after_input();
                    return Flow::Continue;
                };
                match self.engine.click_at(x, y) {
                    Ok(Click::StartScene(start)) => {
                        if let Some(vm) = &self.vm {
                            vm.send(HostCommand::StartScene(start));
                        }
                    }
                    Ok(Click::Handled) => {}
                    Err(e) => eprintln!("[biwa] {e}"),
                }
                self.after_input();
            }
            Event::Advance => {
                self.engine.advance();
                self.after_input();
            }
            Event::Woken => {
                self.drain_images();
                return self.drain_vm();
            }
            Event::Redraw => self.redraw(),
            Event::Shown => {
                // macOS は作った直後のウィンドウがまだ見えておらず、最初のフレームが描けていない。
                if self.stale {
                    self.request_redraw();
                }
            }
            Event::CloseRequested => return Flow::Exit,
        }
        Flow::Continue
    }

    fn frame_done(&mut self) -> bool {
        let animating = self.engine.is_animating();
        if animating && self.last_tick.is_none() {
            // 止まっていたところから動き始めた。ここから時計を進める。
            self.last_tick = Some(Instant::now());
        }
        animating
    }
}

/// ゲームの画面をウィンドウに収める位置と倍率。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Letterbox {
    /// ゲームの論理ピクセル 1 つが物理ピクセルいくつか。
    scale: f32,
    /// 画面を描く範囲 (ウィンドウの中、物理ピクセル)。
    area: Rect,
}

impl Letterbox {
    fn new(screen: (u32, u32), window: WindowSize) -> Self {
        let (sw, sh) = (screen.0.max(1) as f32, screen.1.max(1) as f32);
        let (ww, wh) = (window.width as f32, window.height as f32);
        let scale = (ww / sw).min(wh / sh).max(f32::MIN_POSITIVE);
        let (w, h) = ((sw * scale).round().min(ww), (sh * scale).round().min(wh));
        // 帯の幅は整数にする (文字をピクセルの格子に合わせたまま動かすため)。
        let (x, y) = (((ww - w) / 2.0).floor(), ((wh - h) / 2.0).floor());
        Self { scale, area: Rect::new(x, y, w, h) }
    }

    /// ウィンドウの論理ピクセルの位置を、ゲームの画面の論理ピクセルの位置にする。帯の上なら `None`。
    fn to_screen(self, x: f32, y: f32, window_scale: f32) -> Option<(f32, f32)> {
        let (px, py) = (x * window_scale - self.area.x, y * window_scale - self.area.y);
        let inside = (0.0..=self.area.width).contains(&px) && (0.0..=self.area.height).contains(&py);
        inside.then(|| (px / self.scale, py / self.scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_keeps_the_aspect_ratio_and_centers() {
        // 16:9 の画面を 4:3 のウィンドウに (上下に帯)。
        let fit = Letterbox::new((1280, 720), WindowSize { width: 1600, height: 1200, scale: 2.0 });
        assert_eq!(fit.scale, 1.25);
        assert_eq!(fit.area, Rect::new(0.0, 150.0, 1600.0, 900.0));
        // ウィンドウの論理ピクセル (800, 600) = 物理 (1600, 1200) は画面の右下の角。
        assert_eq!(fit.to_screen(400.0, 300.0, 2.0), Some((640.0, 360.0)));
        assert_eq!(fit.to_screen(800.0, 525.0, 2.0), Some((1280.0, 720.0)));
        // 帯の上。
        assert_eq!(fit.to_screen(400.0, 10.0, 2.0), None);
    }
}
