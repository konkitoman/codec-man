use std::{collections::VecDeque, sync::mpsc};

use cpal::{
    platform::AlsaStream,
    traits::{DeviceTrait, HostTrait},
    Device, Host, SampleRate, Stream, SupportedStreamConfig,
};
use eframe::{egui, App};

pub struct Application {
    host: Host,
    output_device: Device,
    output_device_config: SupportedStreamConfig,
    output_stream: Stream,
    output_sender: mpsc::Sender<Vec<f32>>,
    output_rem_receiver: mpsc::Receiver<usize>,

    buffer: Vec<f32>,

    rem: usize,
    offset: usize,
    length: usize,
    speed: f32,
}

impl Default for Application {
    fn default() -> Self {
        let host = cpal::default_host();
        let output_device = host.default_output_device().unwrap();
        let output_device_config = output_device.default_output_config().unwrap();

        let (output_sender, output_receiver) = mpsc::channel::<Vec<f32>>();
        let (output_rem_sender, output_rem_receiver) = mpsc::channel::<usize>();

        let output_stream = {
            let mut buffer = VecDeque::new();
            output_device
                .build_output_stream(
                    &cpal::StreamConfig {
                        channels: 2,
                        sample_rate: SampleRate(48000),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &mut [f32], info| {
                        while let Ok(buff) = output_receiver.try_recv() {
                            buffer.extend(buff);
                        }
                        for byte in data.iter_mut() {
                            *byte = if let Some(b) = buffer.pop_front() {
                                b
                            } else {
                                0.0
                            }
                        }
                        output_rem_sender.send(buffer.len());
                    },
                    |error| eprintln!("Output stream Error: {error}"),
                    None,
                )
                .unwrap()
        };

        Self {
            host,
            output_device,
            output_device_config,
            output_stream,
            output_sender,
            output_rem_receiver,
            rem: 0,
            offset: 0,
            length: 48 * 20,
            buffer: Vec::new(),
            speed: 1.0,
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        while let Ok(rem) = self.output_rem_receiver.try_recv() {
            self.rem = rem;
        }
        egui::TopBottomPanel::bottom("Controls").show(ctx, |ui| {
            ui.add(egui::widgets::DragValue::new(&mut self.length).prefix("Length"));
            ui.add(egui::DragValue::new(&mut self.offset).prefix("Offset"));
            ui.add(egui::DragValue::new(&mut self.speed).prefix("Speed"));
            if ui.button("Sin").clicked() {
                self.buffer.resize(self.length, 0.0);
                for i in 0..self.length {
                    let sample = i as f32;
                    let sample = (sample * 0.01).sin();
                    self.buffer[i] = sample;
                }
            }

            if ui.button("Clear").clicked() {
                self.buffer.clear();
            }

            if ui.button("Process").clicked() {
                let len = (self.buffer.len() as f32 / self.speed).round() as usize;
                let mut new_buffer = Vec::with_capacity(len);
                for i in 0..len {
                    let original_index = (i as f32 * self.speed).round();
                    let left_index = original_index.floor() as usize;
                    let right_index = left_index + 1;
                    let fractional = original_index.fract();

                    if right_index < self.buffer.len() {
                        let left_sample = self.buffer[left_index];
                        let right_sample = self.buffer[right_index];

                        let interpolate_sample =
                            (1.0 - fractional) * left_sample + fractional * right_sample;
                        new_buffer.push(interpolate_sample);
                    }
                }
                self.buffer = new_buffer;
            }

            if ui.button("Submit").clicked() {
                self.output_sender.send(self.buffer.clone());
                self.rem += 1;
            }
            if self.rem > 0 {
                ui.spinner();
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::plot::Plot::new("Buffer").show(ui, |ui| {
                ui.points(egui::plot::Points::new(
                    egui::plot::PlotPoints::from_ys_f32(&self.buffer),
                ));
                ui.vline(
                    egui::plot::VLine::new(self.buffer.len() as f32 - self.rem as f32).name("Rem"),
                )
            });
        });
    }
}

fn main() {
    eframe::run_native(
        "codec-map",
        eframe::NativeOptions::default(),
        Box::new(|_| Box::<Application>::default()),
    )
    .unwrap();
}
