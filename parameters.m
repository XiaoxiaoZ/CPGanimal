clear
clc
omega1 = 0;
omega2 = 0;
omega3 = 0;
omega = ones(3,3)*0.5;
omega(1,1)=0;
omega(2,2)=0;
omega(3,3)=0;
phi = zeros(3,3);
phi(1,2) = pi/2;
phi(2,3) = pi;
phi(3,2) = pi;
ar = 2;
ax = 2;
R1 = 4;
R2 = 4;
R3 = 4;
X1 = 0;
X2 = 0;
X3 = 0;
rng default % For reproducibility
options = optimoptions('ga','PlotFcn', @gaplotbestf);
x = ga(@(x) animal(x(1),x(2),x(3)),3,options)
%%
omega1 = -64.1138;
omega2 = -281.0089;
omega3 = 93.1018;
simOut=sim('chain_CPG.slx', 100);