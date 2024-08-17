#!/bin/sh
docker run --rm -v "$(pwd)/shaders:/fxc/shaders" gwihlidal/fxc /T vs_4_0 /E vs_main shaders/egui.hlsl /Fo shaders/egui_vs.bin &&
docker run --rm -v "$(pwd)/shaders:/fxc/shaders" gwihlidal/fxc /T ps_4_0 /E ps_main_gamma shaders/egui.hlsl /Fo shaders/egui_ps_gamma.bin &&
docker run --rm -v "$(pwd)/shaders:/fxc/shaders" gwihlidal/fxc /T ps_4_0 /E ps_main_linear shaders/egui.hlsl /Fo shaders/egui_ps_linear.bin