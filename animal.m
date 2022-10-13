function distance = animal(R1,R2,R3,PHI)
%ANIMAL Summary of this function goes here
%   Detailed explanation goes here ,, omega2, omega3, omega, phi, ar, ax, R1, R2, R3, X1, X2, X3

omega1 = 0.6*pi;
omega2 = 0.6*pi;
omega3 = 0.6*pi;
omega = ones(3,3)*2;
phi = ones(3,3)*PHI;
ar = 2;
ax = 2;
R1 = R1;
R2 = R2;
R3 = R3;
X1 = 0;
X2 = 0;
X3 = 0;
%Robot unit length
l=0.5;
% omega2 = omega2;
% omega3 = omega3;
% omega = omega;
% phi = phi;
% ar = ar;
% ax = ax;
% R1 = R1;
% R2 = R2;
% R3 = R3;
% X1 = X1;
% X2 = X2;
% X3 = X3;
options=simset('SrcWorkspace','current','DstWorkspace','base');
simOut=sim('chain_CPG_2021a.slx', 10, options);
%Run simulink simulation
%simOut=sim('CPG.slx',10);
%%Data extraction
outVariable=get(simOut,'yout');
distance = -simOut.distance.Data(end);
PHI
R1
R2
R3
distance
end

