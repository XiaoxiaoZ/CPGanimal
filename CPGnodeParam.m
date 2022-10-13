

%%
rng default % For reproducibility
options = optimoptions('ga','PlotFcn', @gaplotbestf);
result = ga(@(x) Snake5animal(x(1),x(2),x(3),x(4),x(5),x(6)),6,[],[],[],[],[0 0 0 0 0 0],[0.6 0.6 0.6 0.6 0.6 pi],[],options)
