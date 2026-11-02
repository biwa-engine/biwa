//! ウィンドウを出して動かす。
//!
//! 部品をつなぐだけである:
//! - `platform` のイベントループから来るイベントを、エンジン (`core`) への操作に直す
//! - `vm` からの知らせ (syscall) をエンジンに当て、止まる syscall の返事を保留する
//! - エンジンの画像の読み込みの依頼を `image` に渡し、読めた画像をエンジン (大きさ) と `render` (テクスチャ) に渡す
//! - 描き直しが要るときだけ、エンジンの描画の指示を `render` で描く (文字は `text`)

use std::sync::Arc;
use std::time::{Duration, Instant};

use biwa_player_core::engine::{CallStatus, Click, Engine};
use biwa_player_core::geometry::Viewport;
use biwa_player_core::image::{DecodedImage, ImageHandle};
use biwa_player_image::ImageLoader;
use biwa_player_layout::TaffyLayouter;
use biwa_player_platform::{App, Event, Flow, Window, WindowOptions, WindowSize};
use biwa_player_render::Renderer;
use biwa_player_text::TextSystem;
use biwa_player_vm::{CallReply, HostCommand, Vm, VmEvent, VmOptions};

use crate::options::Options;

/// 1 フレームで進める時間の上限。描画が詰まったときに文字送りが一気に飛ばないように。
const MAX_FRAME_STEP: Duration = Duration::from_millis(100);

pub fn run(options: &Options, wasm: Vec<u8>) -> Result<(), String> {
    let window = WindowOptions { title: options.title.clone(), width: 1280.0, height: 720.0 };
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
        Player::new(vm, images, trace)
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
    /// 返事を保留している止まる syscall。
    pending: Option<CallReply>,
    /// 最後にエンジン時計を進めた時刻。動いているものが無い間は `None`。
    last_tick: Option<Instant>,
    trace: bool,
}

impl Player {
    fn new(vm: Option<Vm>, images: Option<ImageLoader>, trace: bool) -> Self {
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
        let (width, height) = size.logical();
        let viewport = Viewport::new(width as f32, height as f32, size.scale as f32);
        let list = self.engine.frame(viewport, &mut self.text, &mut self.layouter);
        if let Err(e) = renderer.render(&list, &mut self.text) {
            eprintln!("[biwa] {e}");
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
        self.drain_vm()
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
