clear
clc
l=1;
omega1 = 0.6*pi;
omega2 = 0.6*pi;
omega3 = 0.6*pi;
omega = ones(3,3)*2;
PHI = 0;
phi = ones(3,3)*PHI;
ar = 2;
ax = 2;
R = 0.3;
R1 = R;
R2 = R;
R3 = R;
X1 = 0;
X2 = 0;
X3 = 0;
%%
rng default % For reproducibility
options = optimoptions('ga','PlotFcn', @gaplotbestf);
x = ga(@(x) animal(x(1),x(2),x(3),x(4)),4,[],[],[],[],[0 0 0 0],[0.6 0.6 0.6 pi],[],options)
%%
R1=0.5708;
R2=0.5348;
R3=0.3731;
PHI=2.9214;
simOut=sim('chain_CPG_2021a.slx', 100);