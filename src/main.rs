use std::{
    collections::VecDeque,
    sync::{mpsc, Mutex, RwLock},
};

use cpal::{
    platform::AlsaStream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Host, SampleRate, Stream, SupportedStreamConfig,
};
use eframe::{egui, App};

pub struct Application {
    host: Host,
    output_device: Device,
    output_device_config: SupportedStreamConfig,
    output_stream: Stream,
    output_sender: mpsc::SyncSender<Vec<f32>>,
    output_rem_receiver: mpsc::Receiver<usize>,
    input_stream: Stream,
    input_receiver: mpsc::Receiver<Vec<f32>>,
    input_sender: mpsc::SyncSender<bool>,

    buffer: Vec<f32>,

    recording: bool,

    rem: usize,
    frequency: f64,
    offset: usize,
    length: usize,
    speed: f32,
}

impl Default for Application {
    fn default() -> Self {
        let host = cpal::default_host();

        let output_device = host.default_output_device().unwrap();
        let output_device_config = output_device.default_output_config().unwrap();

        let input_device = host.default_input_device().unwrap();

        let (output_sender, output_receiver) = mpsc::sync_channel::<Vec<f32>>(16);
        let (output_rem_sender, output_rem_receiver) = mpsc::sync_channel::<usize>(16);
        let (input_sender, input_receiver) = mpsc::sync_channel::<Vec<f32>>(16);
        let (input_sender_rec, input_receiver_rec) = mpsc::sync_channel(16);

        let output_stream = {
            let mut buffer = VecDeque::new();
            output_device
                .build_output_stream(
                    &cpal::StreamConfig {
                        channels: 1,
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

        let input_stream = {
            let mut recording = false;
            input_device
                .build_input_stream(
                    &cpal::StreamConfig {
                        channels: 1,
                        sample_rate: SampleRate(48000),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &[f32], info| {
                        while let Ok(rec) = input_receiver_rec.try_recv() {
                            recording = rec;
                        }
                        if recording {
                            input_sender.send(data.to_vec());
                        }
                    },
                    |error| eprintln!("Input stream Error: {error}"),
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
            frequency: 0.1,
            recording: false,
            input_stream,
            input_receiver,
            input_sender: input_sender_rec,
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        while let Ok(rem) = self.output_rem_receiver.try_recv() {
            self.rem = rem;
        }
        while let Ok(buff) = self.input_receiver.try_recv() {
            if self.recording {
                self.buffer.extend(buff)
            }
        }
        egui::TopBottomPanel::bottom("Controls").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add(egui::widgets::DragValue::new(&mut self.length).prefix("Length: "));
                ui.add(egui::DragValue::new(&mut self.offset).prefix("Offset: "));
                ui.add(egui::DragValue::new(&mut self.speed).prefix("Speed: "));
                ui.add(
                    egui::DragValue::new(&mut self.frequency)
                        .prefix("Freq: ")
                        .speed(0.00001)
                        .clamp_range(0..=1),
                );
            });

            if ui.checkbox(&mut self.recording, "Recording").changed() {
                self.input_sender.send(self.recording);
            }

            if ui.button("Sin").clicked() {
                self.buffer.resize(self.length, 0.0);
                for i in 0..self.length {
                    let sample = i as f32;
                    let sample = (sample as f64 * self.frequency).sin() as f32;
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
            ui.spinner();
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::plot::Plot::new("Buffer").show(ui, |ui| {
                let bounds = ui.plot_bounds();
                let range = bounds.min()[0].max(0.0)..bounds.max()[0].min(self.buffer.len() as f64);
                ui.line(egui::plot::Line::new(
                    egui::plot::PlotPoints::from_parametric_callback(
                        |t| {
                            let i = t as usize;
                            (t, self.buffer[i] as f64)
                        },
                        range.clone(),
                        (range.start as usize..range.end as usize).count(),
                    ),
                ));
                ui.vline(
                    egui::plot::VLine::new(self.buffer.len() as f32 - self.rem as f32).name("Rem"),
                )
            });
        });
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        self.input_stream.pause();
        self.output_stream.pause();
        self.buffer.clear();
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
