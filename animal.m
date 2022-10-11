function distance = animal(omega1, omega2,omega3)
%ANIMAL Summary of this function goes here
%   Detailed explanation goes here ,, omega2, omega3, omega, phi, ar, ax, R1, R2, R3, X1, X2, X3

omega1 = omega1;
omega2 = omega2;
omega3 = omega3;
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
simOut=sim('chain_CPG.slx', 10, options);
%Run simulink simulation
%simOut=sim('CPG.slx',10);
%%Data extraction
outVariable=get(simOut,'yout');
distance = -simOut.distance.Data(end);
disp(omega1)
disp(omega2)
disp(omega3)
disp(distance)
end

