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

%%
for i=1:4
%Run simulink simulation
simOut=sim('CPG.slx',10);
%%Data extraction
outVariable=get(simOut,'yout');
simOut.theta1
if i==2
pause(10)
end
end