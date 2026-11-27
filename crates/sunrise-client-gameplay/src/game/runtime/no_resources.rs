//! A game started without its prepared files (the game build run by hand, not
//! through a launcher): a window that says where to get them
//! (`sunrise_client::no_resources`) and quits when it is closed. No loader, no
//! crash report: nothing went wrong in the game.
use super::*;
use sunrise_client::{
    no_resources::{Action, NoResources},
    window::{GuiFrame, PointerSample, PointerSpace},
};
use winit::event::MouseButton;

struct Missing {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    screen: NoResources,
    pointer: PointerSample,
    window_pointer_space: std::cell::OnceCell<PointerSpace>,
    safe_area: Option<([u32; 2], sunrise_hooks::Insets)>,
    margin_press: bool,
    api: sunrise_client::graphics::GraphicsApi,
    settings: Settings,
    error: Option<Box<dyn std::error::Error>>,
}

/// Shows the screen until the player closes it.
pub fn show(looked_in: &std::path::Path, settings: &Settings) -> Result<()> {
    // The player's language, from the strings compiled in: there are no prepared ones.
    sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load(
        &[],
        &settings.language,
    ));
    let mut app = Missing {
        window: None,
        renderer: None,
        gui: UiResources {
            atlas: HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
                looked_in.to_path_buf(),
            ))?,
            panorama: Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])),
            splashes: Arc::from(""),
            plain_texts: Default::default(),
        },
        screen: NoResources::new(looked_in, sunrise_hooks::engine::get_the_game()),
        pointer: PointerSample::default(),
        window_pointer_space: std::cell::OnceCell::new(),
        safe_area: None,
        margin_press: false,
        api: settings.sunrise.api,
        settings: settings.clone(),
        error: None,
    };

    EventLoop::new()?.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

impl Missing {
    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    #[allow(dead_code)]
    fn projected_pointer(&self) -> sunrise_client::window::GuiPointer {
        self.gui_frame().pointer_to_gui(self.pointer)
    }

    #[inline(always)]
    fn gui_frame(&self) -> GuiFrame {
        self.gui_frame_for(self.size())
    }

    #[inline(always)]
    fn gui_frame_for(&self, size: [u32; 2]) -> GuiFrame {
        let insets = self
            .safe_area
            .filter(|(target, _)| *target == size)
            .map(|(_, insets)| insets);
        let window = self.window.as_deref().map_or_else(
            || PointerSpace::new(size, 1.0),
            |window| {
                *self
                    .window_pointer_space
                    .get_or_init(|| PointerSpace::from_window(window))
            },
        );
        self.settings
            .gui_frame_with_insets(size, insets)
            .with_pointer_space(window)
    }

    /// `f` over a Ui of the window's size, the cursor where the pointer is.
    fn with_ui<R>(
        &self,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        f: impl FnOnce(&mut sunrise_client::ui_theme::Ui) -> R,
    ) -> R {
        let mut ui = sunrise_client::ui_theme::Ui::new(&self.gui.atlas, batch, cursor);
        let result = f(&mut ui);
        ui.finish();
        result
    }

    fn draw(&mut self) {
        if self.renderer.is_some() {
            let target = self.size();
            self.safe_area =
                sunrise_hooks::engine::safe_area(target).map(|insets| (target, insets));
        }
        let frame = self.gui_frame();
        let mut batch = HudBatch::for_frame(&frame);
        let hover = frame.hover(frame.pointer_to_gui(self.pointer));
        self.with_ui(&mut batch, hover, |ui| self.screen.draw(ui));
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch), None) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => log::warn!("Sunrise: frame failed: {e}"),
        }
        // The frame was not shown (the first one often isn't): the glyphs it
        // rasterized never reached the atlas, so draw them again next frame.
        sunrise_client::ui_theme::reset_atlas();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let frame = self.gui_frame();
        let cursor = frame.pointer_to_gui(self.pointer).subpixel();
        if !frame.contains(cursor) {
            return;
        }
        match self.screen.click(&self.gui.atlas, frame, cursor) {
            Some(Action::Open(address)) => packs::open_uri(address),
            Some(Action::Quit) => event_loop.exit(),
            None => {}
        }
    }
}

impl ApplicationHandler for Missing {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        )
        .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer = loading_renderer(
                    window.clone(),
                    false,
                    Some(&self.gui.atlas),
                    Some(&self.gui.panorama),
                    None,
                    self.api,
                )?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            self.window_pointer_space.take();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = self
                    .window_pointer_space
                    .get_or_init(|| PointerSpace::from_window(&window))
                    .os_sample(position);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.margin_press = self.safe_area.is_some() && {
                    let frame = self.gui_frame();
                    !frame.contains(frame.pointer_to_gui(self.pointer).subpixel())
                };
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if !std::mem::take(&mut self.margin_press) {
                    self.click(event_loop);
                }
            }
            WindowEvent::Focused(false) => self.margin_press = false,
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if sunrise_threads::stop_requests() > 0 {
            event_loop.exit();
            return;
        }
        // Wake now and then for Ctrl+C in the console; redraws come with events.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(250),
        ));
    }
}
