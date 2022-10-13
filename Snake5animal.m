function distance = Snake5animal(R1,R2,R3,R4,R5,PHI)
%ANIMAL Summary of this function goes here
%   Detailed explanation goes here ,, omega2, omega3, omega, phi, ar, ax, R1, R2, R3, X1, X2, X3

N = 5;
phiM = ones(N,N)*PHI;
omega = ones(1,N)*0.6*pi;
omegaM = ones(N,N)*2;
ar = 2;
ax = 2;
R = [R1 R2 R3 R4 R5];
X = zeros(1,N);
l=1;
options=simset('SrcWorkspace','current','DstWorkspace','base');
simOut=sim('Snake5.slx', 10, options);
%Run simulink simulation
%simOut=sim('CPG.slx',10);
%%Data extraction
outVariable=get(simOut,'yout');
distance = -simOut.distance.Data(end);
PHI
R
distance
end

